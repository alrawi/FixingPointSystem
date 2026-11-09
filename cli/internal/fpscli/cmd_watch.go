// SPDX-License-Identifier: GPL-3.0-only

package fpscli

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"slices"
	"sort"
	"strings"
	"time"
)

// metricViews are the views of the API contract.
var metricViews = []string{"application", "scenario", "port", "tcp", "tls", "realism"}

// defaultViews are the views that watch follows when --view is absent.
var defaultViews = []string{"application", "scenario", "port"}

// headlineMetrics are the metrics that watch shows when neither --metric nor
// --all-metrics is given. A sample that holds none of them shows all of its
// values, so the views without these names still show something.
var headlineMetrics = []string{
	"sessions.attempted", "sessions.succeeded", "sessions.failed", "sessions.rate", "sessions.concurrent",
	"bytes.tx", "bytes.rx", "throughput.bps",
	"port.throughput.tx_bps", "port.throughput.rx_bps", "port.drops",
}

// flushDelay is how long watch waits for the rest of a tick of the stream
// before it draws what it has.
const flushDelay = 150 * time.Millisecond

// maxStreamFailures is the number of consecutive failed connections after
// which watch gives up.
const maxStreamFailures = 10

// watchOptions are the flags of watch and of run start --watch.
type watchOptions struct {
	views      listFlag
	metrics    listFlag
	side       string
	allMetrics bool
	lines      bool
	poll       bool
	interval   time.Duration
}

func (a *app) watchFlags(fs *flag.FlagSet) *watchOptions {
	wo := &watchOptions{}
	fs.Var(&wo.views, "view", "metrics view to follow, repeatable: "+strings.Join(metricViews, ", ")+" (default "+strings.Join(defaultViews, ", ")+")")
	fs.Var(&wo.metrics, "metric", "metric name to show, repeatable; a name also matches its statistics, such as sessions.response_time.p99")
	fs.StringVar(&wo.side, "side", "client", "side of the sessions that the views count, client or server")
	fs.BoolVar(&wo.allMetrics, "all-metrics", false, "show every metric of a sample, not the headline ones")
	fs.BoolVar(&wo.lines, "lines", false, "print one line per sample instead of a refreshing table, which is the default on a terminal")
	fs.BoolVar(&wo.poll, "poll", false, "poll the metrics view endpoint instead of the event stream")
	fs.DurationVar(&wo.interval, "interval", time.Second, "polling interval of --poll and of the fallback")
	return wo
}

func (a *app) watchCmd(ctx context.Context, args []string) error {
	fs := a.newFlagSet("fps watch")
	wo := a.watchFlags(fs)
	pos, err := parse(fs, args)
	if err != nil {
		return err
	}
	if err := wantArgs("watch", pos, "RUN_ID"); err != nil {
		return err
	}
	if err := checkID("run", pos[0]); err != nil {
		return err
	}
	c, err := a.newClient()
	if err != nil {
		return err
	}
	return a.watch(ctx, c, pos[0], wo)
}

// watch follows a run until it ends and returns the outcome of the run.
func (a *app) watch(ctx context.Context, c *Client, id string, wo *watchOptions) error {
	views := []string(wo.views)
	if len(views) == 0 {
		views = defaultViews
	}
	for _, v := range views {
		if !slices.Contains(metricViews, v) {
			return usagef("--view %q is not one of %s", v, strings.Join(metricViews, ", "))
		}
	}
	if wo.side != "client" && wo.side != "server" {
		return usagef("--side %q is not client or server", wo.side)
	}
	if wo.interval < 100*time.Millisecond {
		return usagef("--interval must be at least 100ms")
	}
	run, raw, err := a.getRun(ctx, c, id)
	if err != nil {
		return err
	}
	d := &display{
		a:     a,
		wo:    wo,
		id:    id,
		table: !a.g.JSON && !wo.lines && a.env.IsTerminal(a.env.Stdout),
		cur:   map[string]MetricSample{},
	}
	if !finalState(run.State) {
		if wo.poll {
			err = a.pollLoop(ctx, c, d, views)
		} else {
			err = a.streamLoop(ctx, c, d, views)
		}
		if err != nil {
			if ctx.Err() != nil {
				a.eprintf("fps: run %s keeps running, stop it with: fps run stop %s\n", id, id)
			}
			return err
		}
		if run, raw, err = a.getRun(ctx, c, id); err != nil {
			return err
		}
	}
	d.final(&run, raw)
	return run.outcome()
}

// display draws the ticks of metrics.
type display struct {
	a     *app
	wo    *watchOptions
	id    string
	table bool
	cur   map[string]MetricSample
}

// tick draws the samples of one tick.
func (d *display) tick(samples []MetricSample) {
	if len(samples) == 0 {
		return
	}
	switch {
	case d.a.g.JSON:
		for _, s := range samples {
			_ = d.emitJSON(map[string]any{"event": "sample", "sample": s})
		}
	case d.table:
		for _, s := range samples {
			d.cur[s.View+"\x00"+s.Key] = s
		}
		d.drawTable()
	default:
		for _, s := range samples {
			d.a.printf("%s %s %s %s\n", dash(s.Ts), s.View, s.Key, d.formatValues(s.Values))
		}
	}
}

func (d *display) emitJSON(v any) error {
	data, err := json.Marshal(v)
	if err != nil {
		return err
	}
	_, err = d.a.env.Stdout.Write(append(data, '\n'))
	return err
}

// selectMetrics returns the names of the values that the display shows.
func (d *display) selectMetrics(values map[string]float64) []string {
	var names []string
	switch {
	case d.wo.allMetrics:
		for n := range values {
			names = append(names, n)
		}
	case len(d.wo.metrics) > 0:
		for n := range values {
			for _, m := range d.wo.metrics {
				if n == m || strings.HasPrefix(n, m+".") {
					names = append(names, n)
					break
				}
			}
		}
	default:
		for _, n := range headlineMetrics {
			if _, ok := values[n]; ok {
				names = append(names, n)
			}
		}
		if len(names) == 0 {
			for n := range values {
				names = append(names, n)
			}
		}
	}
	sort.Strings(names)
	return names
}

func (d *display) formatValues(values map[string]float64) string {
	var parts []string
	for _, n := range d.selectMetrics(values) {
		parts = append(parts, n+"="+fmtNum(values[n]))
	}
	return strings.Join(parts, " ")
}

const clearScreen = "\x1b[H\x1b[2J"

func (d *display) drawTable() {
	keys := make([]string, 0, len(d.cur))
	for k := range d.cur {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	w := d.a.env.Stdout
	_, _ = io.WriteString(w, clearScreen)
	_, _ = fmt.Fprintf(w, "run %s  (Ctrl-C stops watching, the run continues)\n\n", d.id)
	tw := newTable(w)
	_, _ = fmt.Fprintln(tw, "VIEW\tKEY\tMETRIC\tVALUE")
	for _, k := range keys {
		s := d.cur[k]
		for _, n := range d.selectMetrics(s.Values) {
			_, _ = fmt.Fprintf(tw, "%s\t%s\t%s\t%s\n", s.View, s.Key, n, fmtNum(s.Values[n]))
		}
	}
	_ = tw.Flush()
}

// final prints the end of the run.
func (d *display) final(run *Run, raw []byte) {
	if d.a.g.JSON {
		_ = d.emitJSON(map[string]any{"event": "end", "run": json.RawMessage(compactJSON(raw))})
		return
	}
	d.a.printf("\n")
	writeRun(d.a.env.Stdout, run)
}

func compactJSON(raw []byte) []byte {
	var buf bytes.Buffer
	if err := json.Compact(&buf, raw); err != nil {
		return []byte("null")
	}
	return buf.Bytes()
}

// sseEvent is one event of a Server Sent Events stream.
type sseEvent struct {
	id    string
	event string
	data  string
}

// readSSE parses the stream and calls fn for each event that carries data.
func readSSE(r io.Reader, fn func(sseEvent) bool) error {
	sc := bufio.NewScanner(r)
	sc.Buffer(make([]byte, 0, 64<<10), 4<<20)
	var ev sseEvent
	var data []string
	dispatch := func() bool {
		if len(data) > 0 {
			ev.data = strings.Join(data, "\n")
			ok := fn(ev)
			data = nil
			ev = sseEvent{id: ev.id}
			return ok
		}
		ev = sseEvent{id: ev.id}
		return true
	}
	for sc.Scan() {
		line := sc.Text()
		if line == "" {
			if !dispatch() {
				return nil
			}
			continue
		}
		if strings.HasPrefix(line, ":") {
			continue
		}
		field, value, _ := strings.Cut(line, ":")
		value = strings.TrimPrefix(value, " ")
		switch field {
		case "id":
			ev.id = value
		case "event":
			ev.event = value
		case "data":
			data = append(data, value)
		}
	}
	return sc.Err()
}

// streamOnce reads one connection of the event stream. It reports whether the
// stream ended with its end event, and whether it delivered any sample.
func (a *app) streamOnce(ctx context.Context, c *Client, d *display, views []string) (ended, gotData bool, err error) {
	q := url.Values{}
	for _, v := range views {
		q.Add("view", v)
	}
	q.Set("side", d.wo.side)
	resp, err := c.do(ctx, request{method: http.MethodGet, path: "/runs/" + d.id + "/metrics:stream", query: q, accept: "text/event-stream"})
	if err != nil {
		return false, false, err
	}
	defer func() { _ = resp.Body.Close() }()
	if ct := resp.Header.Get("Content-Type"); !strings.HasPrefix(ct, "text/event-stream") {
		return false, false, fmt.Errorf("the metrics stream answered with content type %q, not text/event-stream", ct)
	}
	events := make(chan sseEvent)
	readErr := make(chan error, 1)
	done := make(chan struct{})
	defer close(done)
	go func() {
		err := readSSE(resp.Body, func(ev sseEvent) bool {
			select {
			case events <- ev:
				return true
			case <-done:
				return false
			case <-ctx.Done():
				return false
			}
		})
		readErr <- err
		close(events)
	}()
	var pending []MetricSample
	pendingID := ""
	flush := func() {
		d.tick(pending)
		pending = nil
	}
	timer := time.NewTimer(time.Hour)
	timer.Stop()
	defer timer.Stop()
	for {
		select {
		case ev, ok := <-events:
			if !ok {
				flush()
				if err := <-readErr; err != nil && ctx.Err() == nil {
					return false, gotData, c.netError(err)
				}
				return false, gotData, ctx.Err()
			}
			if ev.event == "end" {
				flush()
				return true, gotData, nil
			}
			if ev.event != "" && ev.event != "message" {
				continue
			}
			var s MetricSample
			if err := json.Unmarshal([]byte(ev.data), &s); err != nil {
				continue
			}
			gotData = true
			if ev.id != pendingID && len(pending) > 0 {
				flush()
			}
			pendingID = ev.id
			pending = append(pending, s)
			timer.Reset(flushDelay)
		case <-timer.C:
			flush()
		case <-ctx.Done():
			return false, gotData, ctx.Err()
		}
	}
}

// streamLoop follows the event stream, reconnects after a drop while the run
// lives, and falls back to polling when the controller has no stream.
func (a *app) streamLoop(ctx context.Context, c *Client, d *display, views []string) error {
	failures := 0
	for {
		ended, gotData, err := a.streamOnce(ctx, c, d, views)
		if ctx.Err() != nil {
			return ctx.Err()
		}
		if ended {
			return nil
		}
		var apiErr *APIError
		if errors.As(err, &apiErr) {
			if apiErr.Status == http.StatusNotImplemented {
				a.eprintf("fps: the controller has no metrics stream, polling every %s instead\n", d.wo.interval)
				return a.pollLoop(ctx, c, d, views)
			}
			if apiErr.Status < 500 {
				return err
			}
		}
		if gotData {
			failures = 0
		}
		run, _, gerr := a.getRun(ctx, c, d.id)
		if gerr == nil && finalState(run.State) {
			return nil
		}
		failures++
		if failures > maxStreamFailures {
			if err == nil {
				err = errors.New("the metrics stream keeps ending before the run does")
			}
			return err
		}
		wait := time.Duration(min(failures, 5)) * time.Second
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(wait):
		}
	}
}

// pollLoop polls the metrics view endpoint and the run until the run ends.
func (a *app) pollLoop(ctx context.Context, c *Client, d *display, views []string) error {
	last := map[string]string{} // newest timestamp per view
	seen := map[string]string{} // newest timestamp per view and key
	for {
		run, _, err := a.getRun(ctx, c, d.id)
		if err != nil {
			return err
		}
		var tick []MetricSample
		for _, v := range views {
			q := url.Values{}
			q.Set("view", v)
			q.Set("side", d.wo.side)
			if t := last[v]; t != "" {
				q.Set("from", t)
			}
			var set struct {
				Samples []MetricSample `json:"samples"`
			}
			if _, err := c.fetchJSON(ctx, request{method: http.MethodGet, path: "/runs/" + d.id + "/metrics", query: q}, &set); err != nil {
				return err
			}
			newest := map[string]MetricSample{}
			for _, s := range set.Samples {
				if prev, ok := newest[s.Key]; !ok || s.Ts >= prev.Ts {
					newest[s.Key] = s
				}
			}
			for k, s := range newest {
				// The window may repeat the newest sample of the last poll.
				if s.Ts <= seen[v+"\x00"+k] && seen[v+"\x00"+k] != "" {
					delete(newest, k)
					continue
				}
				seen[v+"\x00"+k] = s.Ts
				if s.Ts > last[v] {
					last[v] = s.Ts
				}
			}
			keys := make([]string, 0, len(newest))
			for k := range newest {
				keys = append(keys, k)
			}
			sort.Strings(keys)
			for _, k := range keys {
				tick = append(tick, newest[k])
			}
		}
		d.tick(tick)
		if finalState(run.State) {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(d.wo.interval):
		}
	}
}

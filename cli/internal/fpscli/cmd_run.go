// SPDX-License-Identifier: GPL-3.0-only

package fpscli

import (
	"context"
	"encoding/json"
	"net/http"
	"net/url"
	"strconv"
	"strings"
)

// listFlag collects a repeated flag.
type listFlag []string

func (l *listFlag) String() string { return strings.Join(*l, ",") }

func (l *listFlag) Set(v string) error {
	*l = append(*l, v)
	return nil
}

func (a *app) runCmd(ctx context.Context, args []string) error {
	sub, rest, err := subcommand("run", args, "start", "list", "show", "stop")
	if err != nil {
		return err
	}
	switch sub {
	case "start":
		return a.runStart(ctx, rest)
	case "list":
		return a.runList(ctx, rest)
	case "show":
		return a.runShow(ctx, rest)
	default:
		return a.runStop(ctx, rest)
	}
}

func (a *app) runStart(ctx context.Context, args []string) error {
	fs := a.newFlagSet("fps run start")
	var agents, labels listFlag
	fs.Var(&agents, "agent", "agent `ID` to run on, repeatable; the controller picks reserved agents when absent")
	fs.Var(&labels, "label", "run label as `KEY=VALUE`, repeatable")
	follow := fs.Bool("watch", false, "follow the live metrics until the run ends, and exit by its outcome")
	wo := a.watchFlags(fs)
	pos, err := parse(fs, args)
	if err != nil {
		return err
	}
	if err := wantArgs("run start", pos, "CONFIG_ID"); err != nil {
		return err
	}
	if err := checkID("configuration", pos[0]); err != nil {
		return err
	}
	for _, id := range agents {
		if err := checkID("agent", id); err != nil {
			return err
		}
	}
	body := map[string]any{"config_id": pos[0]}
	if len(agents) > 0 {
		body["agents"] = []string(agents)
	}
	if len(labels) > 0 {
		m := map[string]string{}
		for _, l := range labels {
			k, v, ok := strings.Cut(l, "=")
			if !ok || k == "" {
				return usagef("--label %q is not KEY=VALUE", l)
			}
			m[k] = v
		}
		body["labels"] = m
	}
	data, err := json.Marshal(body)
	if err != nil {
		return err
	}
	c, err := a.newClient()
	if err != nil {
		return err
	}
	var run Run
	raw, err := c.fetchJSON(ctx, request{method: http.MethodPost, path: "/runs", body: data, ctype: "application/json"}, &run)
	if err != nil {
		return err
	}
	if *follow {
		if !a.g.JSON {
			a.printf("started run %s (%s)\n", run.ID, run.State)
		}
		return a.watch(ctx, c, run.ID, wo)
	}
	if a.g.JSON {
		printJSON(a.env.Stdout, raw)
		return nil
	}
	a.printf("%s\n", run.ID)
	a.eprintf("started run %s, state %s\n", run.ID, run.State)
	return nil
}

func (a *app) runList(ctx context.Context, args []string) error {
	fs := a.newFlagSet("fps run list")
	state := fs.String("state", "", "only runs in this state: pending, preparing, running, stopping, completed, failed, or aborted")
	size := fs.Int("page-size", 0, "runs per page, 1 to 500 (default 50)")
	token := fs.String("page-token", "", "`TOKEN` of the page to fetch, from a previous next_page_token")
	all := fs.Bool("all", false, "follow every page")
	pos, err := parse(fs, args)
	if err != nil {
		return err
	}
	if err := wantArgs("run list", pos); err != nil {
		return err
	}
	if *state != "" {
		switch *state {
		case "pending", "preparing", "running", "stopping", "completed", "failed", "aborted":
		default:
			return usagef("--state %q is not a run state", *state)
		}
	}
	if *size < 0 || *size > 500 {
		return usagef("--page-size must be between 1 and 500")
	}
	c, err := a.newClient()
	if err != nil {
		return err
	}
	var runs []json.RawMessage
	var typed []Run
	next := *token
	for {
		q := url.Values{}
		if *state != "" {
			q.Set("state", *state)
		}
		if *size > 0 {
			q.Set("page_size", strconv.Itoa(*size))
		}
		if next != "" {
			q.Set("page_token", next)
		}
		var page struct {
			Items         []json.RawMessage `json:"items"`
			NextPageToken string            `json:"next_page_token"`
		}
		if _, err := c.fetchJSON(ctx, request{method: http.MethodGet, path: "/runs", query: q}, &page); err != nil {
			return err
		}
		runs = append(runs, page.Items...)
		for _, it := range page.Items {
			var r Run
			if err := json.Unmarshal(it, &r); err != nil {
				return err
			}
			typed = append(typed, r)
		}
		next = page.NextPageToken
		if next == "" || !*all {
			break
		}
	}
	if a.g.JSON {
		out := map[string]any{"items": runs}
		if runs == nil {
			out["items"] = []json.RawMessage{}
		}
		if next != "" {
			out["next_page_token"] = next
		}
		return printValue(a.env.Stdout, out)
	}
	if len(typed) == 0 {
		a.eprintf("no runs\n")
		return nil
	}
	writeRunTable(a.env.Stdout, typed)
	if next != "" {
		a.eprintf("more runs follow, pass --page-token %s or --all\n", next)
	}
	return nil
}

func (a *app) getRun(ctx context.Context, c *Client, id string) (Run, []byte, error) {
	var r Run
	raw, err := c.fetchJSON(ctx, request{method: http.MethodGet, path: "/runs/" + id}, &r)
	return r, raw, err
}

func (a *app) runShow(ctx context.Context, args []string) error {
	fs := a.newFlagSet("fps run show")
	pos, err := parse(fs, args)
	if err != nil {
		return err
	}
	if err := wantArgs("run show", pos, "ID"); err != nil {
		return err
	}
	if err := checkID("run", pos[0]); err != nil {
		return err
	}
	c, err := a.newClient()
	if err != nil {
		return err
	}
	r, raw, err := a.getRun(ctx, c, pos[0])
	if err != nil {
		return err
	}
	if a.g.JSON {
		printJSON(a.env.Stdout, raw)
		return nil
	}
	writeRun(a.env.Stdout, &r)
	return nil
}

func (a *app) runStop(ctx context.Context, args []string) error {
	fs := a.newFlagSet("fps run stop")
	force := fs.Bool("force", false, "abort at once and skip the ramp down")
	pos, err := parse(fs, args)
	if err != nil {
		return err
	}
	if err := wantArgs("run stop", pos, "ID"); err != nil {
		return err
	}
	if err := checkID("run", pos[0]); err != nil {
		return err
	}
	c, err := a.newClient()
	if err != nil {
		return err
	}
	body := []byte(`{}`)
	if *force {
		body = []byte(`{"force":true}`)
	}
	var r Run
	raw, err := c.fetchJSON(ctx, request{method: http.MethodPost, path: "/runs/" + pos[0] + ":stop", body: body, ctype: "application/json"}, &r)
	if err != nil {
		return err
	}
	if a.g.JSON {
		printJSON(a.env.Stdout, raw)
		return nil
	}
	a.printf("run %s is %s\n", r.ID, r.State)
	return nil
}

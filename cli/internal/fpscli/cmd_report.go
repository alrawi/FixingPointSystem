// SPDX-License-Identifier: GPL-3.0-only

package fpscli

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"os"
)

// reportFormats maps a --format value to the format query parameter and the
// media type that the client accepts. "text" is a summary that the client
// renders from the JSON report.
var reportFormats = map[string]struct{ query, accept string }{
	"text": {"json", "application/json"},
	"json": {"json", "application/json"},
	"csv":  {"csv", "text/csv"},
	"html": {"html", "text/html"},
}

func (a *app) reportCmd(ctx context.Context, args []string) error {
	fs := a.newFlagSet("fps report")
	format := fs.String("format", "text", "report format: text (a summary), json, csv, or html")
	out := fs.String("output", "", "write the report to this `FILE` instead of standard output")
	fs.StringVar(out, "o", "", "short for --output")
	pos, err := parse(fs, args)
	if err != nil {
		return err
	}
	if err := wantArgs("report", pos, "RUN_ID"); err != nil {
		return err
	}
	if err := checkID("run", pos[0]); err != nil {
		return err
	}
	f, ok := reportFormats[*format]
	if !ok {
		return usagef("--format %q is not one of text, json, csv, html", *format)
	}
	if a.g.JSON && *format == "text" {
		*format = "json"
	}
	c, err := a.newClient()
	if err != nil {
		return err
	}
	q := url.Values{}
	q.Set("format", f.query)
	data, err := c.fetch(ctx, request{method: http.MethodGet, path: "/runs/" + pos[0] + "/report", query: q, accept: f.accept})
	if err != nil {
		return err
	}
	if *out != "" {
		// The report is meant to be shared, so the file is not private.
		if err := os.WriteFile(*out, reportBytes(*format, data), 0o644); err != nil { //nolint:gosec // G306: reports are shared
			return fmt.Errorf("write the report: %w", err)
		}
		a.eprintf("wrote %s\n", *out)
		return nil
	}
	if *format == "text" {
		var rep RunReport
		if err := json.Unmarshal(data, &rep); err != nil {
			return fmt.Errorf("decode the report: %w", err)
		}
		a.writeReport(&rep)
		return nil
	}
	_, err = a.env.Stdout.Write(reportBytes(*format, data))
	return err
}

// reportBytes returns the bytes to print for a format: JSON is indented, and
// text prints as JSON when it goes to a file.
func reportBytes(format string, data []byte) []byte {
	if format != "json" && format != "text" {
		return data
	}
	var buf bytes.Buffer
	printJSON(&buf, data)
	return buf.Bytes()
}

func (a *app) writeReport(rep *RunReport) {
	writeRun(a.env.Stdout, &rep.Run)
	if len(rep.Realism) == 0 {
		return
	}
	a.printf("\nRealism:\n")
	tw := newTable(a.env.Stdout)
	_, _ = fmt.Fprintln(tw, "  RESULT\tPROPERTY\tOBSERVED\tTARGET")
	for _, r := range rep.Realism {
		res := "fail"
		if r.Passed {
			res = "pass"
		}
		_, _ = fmt.Fprintf(tw, "  %s\t%s\t%s\t%s\n", res, r.Property, fmtNum(r.Observed), r.Target)
	}
	_ = tw.Flush()
}

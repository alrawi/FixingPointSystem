// SPDX-License-Identifier: GPL-3.0-only

package fpscli

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

// maxConfig bounds the size of a configuration that the client uploads.
const maxConfig = 16 << 20

func (a *app) configCmd(ctx context.Context, args []string) error {
	sub, rest, err := subcommand("config", args, "validate", "upload", "list", "show")
	if err != nil {
		return err
	}
	switch sub {
	case "validate":
		return a.configValidate(ctx, rest)
	case "upload":
		return a.configUpload(ctx, rest)
	case "list":
		return a.configList(ctx, rest)
	default:
		return a.configShow(ctx, rest)
	}
}

// readDocument reads the configuration named by arg, or standard input for
// "-", and picks its content type.
func (a *app) readDocument(arg, format string) (body []byte, ctype, source string, err error) {
	var data []byte
	source = arg
	if arg == "-" {
		source = "stdin"
		data, err = io.ReadAll(io.LimitReader(a.env.Stdin, maxConfig+1))
	} else {
		data, err = os.ReadFile(arg) //nolint:gosec // G304: the user names the file
	}
	if err != nil {
		return nil, "", "", usagef("read the configuration: %v", err)
	}
	if len(data) > maxConfig {
		return nil, "", "", usagef("the configuration is larger than %d MiB", maxConfig>>20)
	}
	if len(strings.TrimSpace(string(data))) == 0 {
		return nil, "", "", usagef("the configuration %s is empty", source)
	}
	if source != "stdin" {
		source = filepath.Base(arg)
	}
	switch format {
	case "json":
		ctype = "application/json"
	case "yaml":
		ctype = "application/yaml"
	case "":
		switch strings.ToLower(filepath.Ext(arg)) {
		case ".json":
			ctype = "application/json"
		case ".yaml", ".yml":
			ctype = "application/yaml"
		default:
			// The controller parses the two formats by content type, so a
			// document that starts with a brace is JSON.
			if strings.HasPrefix(strings.TrimSpace(string(data)), "{") {
				ctype = "application/json"
			} else {
				ctype = "application/yaml"
			}
		}
	default:
		return nil, "", "", usagef("--format %q is not json or yaml", format)
	}
	return data, ctype, source, nil
}

// upload stores a configuration and records it in the history.
func (a *app) upload(ctx context.Context, c *Client, data []byte, ctype, source string) (ConfigRef, []byte, error) {
	var ref ConfigRef
	raw, err := c.fetchJSON(ctx, request{method: http.MethodPost, path: "/configs", body: data, ctype: ctype}, &ref)
	if err != nil {
		return ref, nil, err
	}
	a.recordUpload(ref, source)
	return ref, raw, nil
}

func (a *app) configUpload(ctx context.Context, args []string) error {
	fs := a.newFlagSet("fps config upload")
	format := fs.String("format", "", "document format, json or yaml; by default the file extension or the first character decides")
	pos, err := parse(fs, args)
	if err != nil {
		return err
	}
	if err := wantArgs("config upload", pos, "FILE|-"); err != nil {
		return err
	}
	data, ctype, source, err := a.readDocument(pos[0], *format)
	if err != nil {
		return err
	}
	c, err := a.newClient()
	if err != nil {
		return err
	}
	ref, raw, err := a.upload(ctx, c, data, ctype, source)
	if err != nil {
		return err
	}
	if a.g.JSON {
		printJSON(a.env.Stdout, raw)
		return nil
	}
	a.printf("%s\n", ref.ID)
	a.eprintf("stored configuration %s (%s)\n", ref.ID, ref.Name)
	return nil
}

func (a *app) configValidate(ctx context.Context, args []string) error {
	fs := a.newFlagSet("fps config validate")
	id := fs.String("id", "", "validate the stored configuration with this identifier instead of uploading a file")
	format := fs.String("format", "", "document format, json or yaml")
	pos, err := parse(fs, args)
	if err != nil {
		return err
	}
	if *id != "" {
		if len(pos) > 0 {
			return usagef("fps config validate takes --id or a file, not both")
		}
		if err := checkID("configuration", *id); err != nil {
			return err
		}
	} else if err := wantArgs("config validate", pos, "FILE|- or --id ID"); err != nil {
		return err
	}
	var data []byte
	var ctype, source string
	if *id == "" {
		if data, ctype, source, err = a.readDocument(pos[0], *format); err != nil {
			return err
		}
	}
	c, err := a.newClient()
	if err != nil {
		return err
	}
	configID := *id
	if configID == "" {
		// The API has no dry run, so validation stores the configuration first.
		ref, _, err := a.upload(ctx, c, data, ctype, source)
		var apiErr *APIError
		if errors.As(err, &apiErr) && (apiErr.Status == http.StatusUnprocessableEntity || apiErr.Status == http.StatusBadRequest) {
			// The upload refused the document: it is invalid.
			a.eprintf("%s\n", apiErr.Error())
			apiErr.printFindings(a.env.Stderr)
			return &outcomeError{msg: "the configuration is invalid and was not stored"}
		}
		if err != nil {
			return err
		}
		configID = ref.ID
		a.eprintf("stored configuration %s (%s)\n", ref.ID, ref.Name)
	}
	var rep ValidationReport
	raw, err := c.fetchJSON(ctx, request{method: http.MethodPost, path: "/configs/" + configID + ":validate"}, &rep)
	if err != nil {
		return err
	}
	if a.g.JSON {
		printJSON(a.env.Stdout, raw)
	} else {
		a.writeValidation(configID, &rep)
	}
	if !rep.Valid {
		return &outcomeError{msg: "the configuration " + configID + " is invalid"}
	}
	return nil
}

func (a *app) writeValidation(id string, rep *ValidationReport) {
	if rep.Valid {
		a.printf("configuration %s is valid\n", id)
	} else {
		a.printf("configuration %s is invalid\n", id)
	}
	if rep.Capacity != nil && rep.Capacity.AgentsRequired != nil && rep.Capacity.AgentsAvailable != nil {
		a.printf("agents: %d required, %d available\n", *rep.Capacity.AgentsRequired, *rep.Capacity.AgentsAvailable)
	}
	if len(rep.Findings) > 0 {
		a.printf("\n")
		printFindings(a.env.Stdout, rep.Findings)
	}
}

func (a *app) configList(ctx context.Context, args []string) error {
	fs := a.newFlagSet("fps config list")
	size := fs.Int("page-size", 0, "configurations per page, 1 to 500 (default 50)")
	token := fs.String("page-token", "", "`TOKEN` of the page to fetch, from a previous next_page_token")
	all := fs.Bool("all", false, "follow every page")
	allControllers := fs.Bool("all-controllers", false, "list the uploads of this machine to every controller, from the history file, not only to --url")
	pos, err := parse(fs, args)
	if err != nil {
		return err
	}
	if err := wantArgs("config list", pos); err != nil {
		return err
	}
	if *size < 0 || *size > 500 {
		return usagef("--page-size must be between 1 and 500")
	}
	if *allControllers {
		return a.configListHistory(true, "")
	}
	c, err := a.newClient()
	if err != nil {
		return err
	}
	var items []json.RawMessage
	var typed []ConfigRef
	next := *token
	for first := true; ; first = false {
		q := url.Values{}
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
		if _, err := c.fetchJSON(ctx, request{method: http.MethodGet, path: "/configs", query: q}, &page); err != nil {
			// Only a controller that answers 501 lacks the operation. Any
			// other failure is the answer, so the history never hides it.
			var apiErr *APIError
			if first && errors.As(err, &apiErr) && apiErr.Status == http.StatusNotImplemented {
				return a.configListHistory(false, "this controller does not serve listConfigs, so this lists the uploads of this machine")
			}
			return err
		}
		items = append(items, page.Items...)
		for _, it := range page.Items {
			var ref ConfigRef
			if err := json.Unmarshal(it, &ref); err != nil {
				return err
			}
			typed = append(typed, ref)
		}
		next = page.NextPageToken
		if next == "" || !*all {
			break
		}
	}
	if a.g.JSON {
		out := map[string]any{"items": items}
		if items == nil {
			out["items"] = []json.RawMessage{}
		}
		if next != "" {
			out["next_page_token"] = next
		}
		return printValue(a.env.Stdout, out)
	}
	if len(typed) == 0 {
		a.eprintf("the controller stores no configuration yet\n")
		return nil
	}
	tw := newTable(a.env.Stdout)
	_, _ = tw.Write([]byte("ID\tNAME\tCREATED\n"))
	for _, r := range typed {
		_, _ = tw.Write([]byte(r.ID + "\t" + r.Name + "\t" + dash(r.CreatedAt) + "\n"))
	}
	_ = tw.Flush()
	if next != "" {
		a.eprintf("more configurations follow, pass --page-token %s or --all\n", next)
	}
	return nil
}

// configListHistory lists the uploads that the history file records. It is
// the fallback for a controller that answers listConfigs with 501, and the
// only way to list the uploads to several controllers. A non empty note goes
// to standard error after the list.
func (a *app) configListHistory(all bool, note string) error {
	h, err := a.readHistory()
	if err != nil {
		return usagef("read the history file %s: %v", a.historyPath(), err)
	}
	var shown []historyEntry
	for _, e := range h.Configs {
		if all || e.Controller == a.g.URL {
			shown = append(shown, e)
		}
	}
	if a.g.JSON {
		if shown == nil {
			shown = []historyEntry{}
		}
		return printValue(a.env.Stdout, map[string]any{"items": shown})
	}
	if len(shown) == 0 {
		a.eprintf("no configuration was uploaded from this machine to %s yet\n", a.g.URL)
		if note != "" {
			a.eprintf("%s\n", note)
		}
		return nil
	}
	tw := newTable(a.env.Stdout)
	hdr := "ID\tNAME\tUPLOADED\tSOURCE"
	if all {
		hdr += "\tCONTROLLER"
	}
	_, _ = tw.Write([]byte(hdr + "\n"))
	for _, e := range shown {
		line := e.ID + "\t" + e.Name + "\t" + e.UploadedAt + "\t" + dash(e.Source)
		if all {
			line += "\t" + e.Controller
		}
		_, _ = tw.Write([]byte(line + "\n"))
	}
	_ = tw.Flush()
	if note != "" {
		a.eprintf("%s\n", note)
	}
	return nil
}

func (a *app) configShow(ctx context.Context, args []string) error {
	fs := a.newFlagSet("fps config show")
	pos, err := parse(fs, args)
	if err != nil {
		return err
	}
	if err := wantArgs("config show", pos, "ID"); err != nil {
		return err
	}
	if err := checkID("configuration", pos[0]); err != nil {
		return err
	}
	c, err := a.newClient()
	if err != nil {
		return err
	}
	raw, err := c.fetchJSON(ctx, request{method: http.MethodGet, path: "/configs/" + pos[0]}, nil)
	if err != nil {
		return err
	}
	printJSON(a.env.Stdout, raw)
	return nil
}

// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"path/filepath"
	"slices"
	"testing"
	"time"

	"github.com/getkin/kin-openapi/openapi3filter"
	"github.com/getkin/kin-openapi/routers"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/api/contract"
)

// listPage is the body of listConfigs.
type listPage struct {
	Items []api.ConfigRef `json:"items"`
	Next  *string         `json:"next_page_token"`
}

// listResult is one exchange with listConfigs.
type listResult struct {
	status int
	header http.Header
	body   []byte
}

// callList sends GET /v1/configs with the raw query and validates the
// response against the OpenAPI document, and the request too when validReq
// holds.
func callList(t *testing.T, h *Handler, query string, validReq bool) listResult {
	t.Helper()
	srv := testServer(t, h)
	path := "/v1/configs"
	if query != "" {
		path += "?" + query
	}
	resp, data := do(t, srv, http.MethodGet, path, readToken, "", nil)

	doc, err := contract.Load(filepath.Join(contracts, "openapi", "fps-api-v1.yaml"))
	if err != nil {
		t.Fatal(err)
	}
	item := doc.Paths.Value("/configs")
	if item == nil || item.GetOperation(http.MethodGet) == nil {
		t.Fatal("the contract has no GET /configs")
	}
	req, err := http.NewRequestWithContext(context.Background(), http.MethodGet, srv.URL+path, nil)
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", "Bearer "+readToken)
	in := &openapi3filter.RequestValidationInput{
		Request: req,
		Route:   &routers.Route{Spec: doc, Path: "/configs", PathItem: item, Method: http.MethodGet, Operation: item.GetOperation(http.MethodGet)},
		Options: &openapi3filter.Options{AuthenticationFunc: openapi3filter.NoopAuthenticationFunc},
	}
	if validReq {
		if err := openapi3filter.ValidateRequest(context.Background(), in); err != nil {
			t.Fatalf("GET %s: request breaks the contract: %v", path, err)
		}
	}
	out := &openapi3filter.ResponseValidationInput{
		RequestValidationInput: in,
		Status:                 resp.StatusCode,
		Header:                 resp.Header,
		Body:                   io.NopCloser(bytes.NewReader(data)),
		Options:                &openapi3filter.Options{IncludeResponseStatus: true},
	}
	if err := openapi3filter.ValidateResponse(context.Background(), out); err != nil {
		t.Fatalf("GET %s: response breaks the contract: %v\nbody: %s", path, err, data)
	}
	return listResult{status: resp.StatusCode, header: resp.Header, body: data}
}

func (r listResult) page(t *testing.T) listPage {
	t.Helper()
	if r.status != http.StatusOK {
		t.Fatalf("status %d, body %s", r.status, r.body)
	}
	var p listPage
	if err := json.Unmarshal(r.body, &p); err != nil {
		t.Fatalf("decode %s: %v", r.body, err)
	}
	if p.Items == nil {
		t.Fatalf("items is absent in %s", r.body)
	}
	return p
}

func refIDs(items []api.ConfigRef) []string {
	out := make([]string, len(items))
	for i, r := range items {
		out[i] = r.Id
	}
	return out
}

// storeConfigs stores n configurations through the handler, one second apart,
// and returns their ids, oldest first.
func storeConfigs(t *testing.T, h *Handler, n int) []string {
	t.Helper()
	srv := testServer(t, h)
	data := exampleNamed(t, "enterprise_mix_10g").data
	var ids []string
	for range n {
		resp, body := do(t, srv, http.MethodPost, "/v1/configs", runToken, "application/yaml", data)
		if resp.StatusCode != http.StatusCreated {
			t.Fatalf("create: status %d, body %s", resp.StatusCode, body)
		}
		var ref api.ConfigRef
		if err := json.Unmarshal(body, &ref); err != nil {
			t.Fatal(err)
		}
		ids = append(ids, ref.Id)
	}
	return ids
}

// newestFirst returns ids reversed.
func newestFirst(ids []string) []string {
	out := slices.Clone(ids)
	slices.Reverse(out)
	return out
}

func TestListConfigsEmpty(t *testing.T) {
	h := newTestHandler(t, openMemory(t), Options{})
	p := callList(t, h, "", true).page(t)
	if len(p.Items) != 0 || p.Next != nil {
		t.Fatalf("empty list = %+v", p)
	}
}

func TestListConfigsOrderAndPaging(t *testing.T) {
	h := newTestHandler(t, openMemory(t), Options{})
	stored := storeConfigs(t, h, 5)

	// Without parameters: every configuration, newest first, no further page.
	p := callList(t, h, "", true).page(t)
	if got := refIDs(p.Items); !slices.Equal(got, newestFirst(stored)) || p.Next != nil {
		t.Fatalf("list = %v next %v, want %v and no next", got, p.Next, newestFirst(stored))
	}
	for _, it := range p.Items {
		if it.Name != "enterprise_mix_10g" || it.CreatedAt == nil || !it.CreatedAt.Equal(fixedNow) {
			t.Errorf("item %+v lacks the stored name or time", it)
		}
	}

	// Paging by two walks every configuration once, newest first.
	var walked []string
	query := "page_size=2"
	for pages := 0; ; pages++ {
		if pages > 5 {
			t.Fatalf("paging does not end after %v", walked)
		}
		page := callList(t, h, query, true).page(t)
		if len(page.Items) > 2 {
			t.Fatalf("page of %d items, want at most 2", len(page.Items))
		}
		walked = append(walked, refIDs(page.Items)...)
		if page.Next == nil {
			break
		}
		if len(page.Items) != 2 {
			t.Fatalf("a short page of %d carries a token", len(page.Items))
		}
		query = "page_size=2&page_token=" + *page.Next
	}
	if !slices.Equal(walked, newestFirst(stored)) {
		t.Fatalf("paged walk = %v, want %v", walked, newestFirst(stored))
	}

	// A page that ends exactly at the oldest configuration has no token.
	if page := callList(t, h, "page_size=5", true).page(t); page.Next != nil || len(page.Items) != 5 {
		t.Fatalf("page_size=5 = %v next %v", refIDs(page.Items), page.Next)
	}

	// A configuration stored between two pages neither shifts nor repeats an item.
	first := callList(t, h, "page_size=2", true).page(t)
	extra := storeConfigs(t, h, 1)[0]
	second := callList(t, h, "page_size=2&page_token="+*first.Next, true).page(t)
	if want := []string{stored[2], stored[1]}; !slices.Equal(refIDs(second.Items), want) {
		t.Fatalf("second page after a new config = %v, want %v", refIDs(second.Items), want)
	}
	if fresh := callList(t, h, "page_size=1", true).page(t); len(fresh.Items) != 1 || fresh.Items[0].Id != extra {
		t.Fatalf("newest config = %v, want %s", refIDs(fresh.Items), extra)
	}
}

func TestListConfigsRejectsBadParameters(t *testing.T) {
	h := newTestHandler(t, openMemory(t), Options{})
	for _, query := range []string{
		"page_size=0", "page_size=501", "page_size=-1", "page_size=many",
		"page_token=%21%21", "page_token=Y2ZnXzE", "page_token=YzA", "page_token=cjE",
	} {
		t.Run(query, func(t *testing.T) {
			res := callList(t, h, query, false)
			decodeProblem(t, &http.Response{StatusCode: res.status, Header: res.header}, res.body, http.StatusBadRequest)
		})
	}
}

func TestListConfigsNeedsReadScope(t *testing.T) {
	srv := testServer(t, newTestHandler(t, openMemory(t), Options{}))
	resp, data := do(t, srv, http.MethodGet, "/v1/configs", "", "", nil)
	decodeProblem(t, resp, data, http.StatusUnauthorized)
}

func TestSQLiteListOrderAndPaging(t *testing.T) {
	ctx := context.Background()
	s := openMemory(t)
	for i := 1; i <= 5; i++ {
		rec := Record{ID: fmt.Sprintf("cfg_%d", i), Name: "n", Body: []byte(`{}`), CreatedAt: time.Unix(int64(100-i), 0)}
		if err := s.Create(ctx, rec); err != nil {
			t.Fatal(err)
		}
	}
	// The order follows creation, not the clock: cfg_5 has the oldest time.
	page, err := s.List(ctx, ListOptions{Limit: 2})
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Items) != 2 || page.Items[0].ID != "cfg_5" || page.Items[1].ID != "cfg_4" || page.Next != page.Items[1].Seq {
		t.Fatalf("first page = %+v", page)
	}
	page, err = s.List(ctx, ListOptions{Limit: 10, After: page.Next})
	if err != nil {
		t.Fatal(err)
	}
	if got := summaryIDs(page.Items); !slices.Equal(got, []string{"cfg_3", "cfg_2", "cfg_1"}) || page.Next != 0 {
		t.Fatalf("second page = %v next %d", got, page.Next)
	}
	if _, err := s.List(ctx, ListOptions{Limit: 0}); err == nil {
		t.Error("List with limit 0 succeeded")
	}
}

func summaryIDs(items []Summary) []string {
	out := make([]string, len(items))
	for i, s := range items {
		out[i] = s.ID
	}
	return out
}

// TestSQLiteListSurvivesRestart checks that the creation order persists in the
// database file.
func TestSQLiteListSurvivesRestart(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "configs.db")
	s, err := Open(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{"b", "a", "c"} {
		if err := s.Create(ctx, Record{ID: id, Name: id, Body: []byte(`{}`), CreatedAt: fixedNow}); err != nil {
			t.Fatal(err)
		}
	}
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}
	s, err = Open(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	page, err := s.List(ctx, ListOptions{Limit: 10})
	if err != nil {
		t.Fatal(err)
	}
	if got := summaryIDs(page.Items); !slices.Equal(got, []string{"c", "a", "b"}) {
		t.Fatalf("after restart = %v, want c, a, b", got)
	}
	if err := s.Create(ctx, Record{ID: "d", Name: "d", Body: []byte(`{}`), CreatedAt: fixedNow}); err != nil {
		t.Fatal(err)
	}
	if page, err = s.List(ctx, ListOptions{Limit: 1}); err != nil || page.Items[0].ID != "d" {
		t.Fatalf("newest after restart = %+v, %v", page, err)
	}
}

// TestMigrationFromVersionOne checks that a database of the first table layout
// keeps its configurations in creation order and takes new ones.
func TestMigrationFromVersionOne(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "v1.db")
	db, err := sql.Open("sqlite", "file:"+path)
	if err != nil {
		t.Fatal(err)
	}
	for _, stmt := range append(slices.Clone(migrations[0]),
		`INSERT INTO configs (id, name, body, created_at) VALUES ('zz', 'first', '{}', '2026-11-01T00:00:00Z')`,
		`INSERT INTO configs (id, name, body, created_at) VALUES ('aa', 'second', '{}', '2026-11-02T00:00:00Z')`,
		`PRAGMA user_version = 1`) {
		if _, err := db.ExecContext(ctx, stmt); err != nil {
			t.Fatal(err)
		}
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	s, err := Open(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	if err := s.Create(ctx, Record{ID: "mm", Name: "third", Body: []byte(`{}`), CreatedAt: fixedNow}); err != nil {
		t.Fatal(err)
	}
	if err := s.Create(ctx, Record{ID: "mm", Name: "again", Body: []byte(`{}`), CreatedAt: fixedNow}); err == nil {
		t.Error("a duplicate id was stored")
	}
	page, err := s.List(ctx, ListOptions{Limit: 10})
	if err != nil {
		t.Fatal(err)
	}
	if got := summaryIDs(page.Items); !slices.Equal(got, []string{"mm", "aa", "zz"}) {
		t.Fatalf("after migration = %v, want mm, aa, zz", got)
	}
	got, err := s.Get(ctx, "zz")
	if err != nil || got.Name != "first" || !got.CreatedAt.Equal(time.Date(2026, 11, 1, 0, 0, 0, 0, time.UTC)) {
		t.Fatalf("Get zz = %+v, %v", got, err)
	}
}

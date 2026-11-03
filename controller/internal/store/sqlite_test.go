// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"bytes"
	"context"
	"database/sql"
	"errors"
	"fmt"
	"path/filepath"
	"sync"
	"testing"
	"time"
)

func openMemory(t *testing.T) *SQLite {
	t.Helper()
	s, err := Open(context.Background(), MemoryPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	return s
}

func TestSQLiteCreateAndGet(t *testing.T) {
	ctx := context.Background()
	s := openMemory(t)
	at := time.Date(2026, 11, 3, 12, 0, 0, 123456789, time.FixedZone("x", 3600))
	rec := Record{ID: "cfg_1", Name: "http_smoke", Body: []byte(`{"a":1}`), CreatedAt: at}
	if err := s.Create(ctx, rec); err != nil {
		t.Fatal(err)
	}
	got, err := s.Get(ctx, "cfg_1")
	if err != nil {
		t.Fatal(err)
	}
	if got.ID != rec.ID || got.Name != rec.Name || !bytes.Equal(got.Body, rec.Body) || !got.CreatedAt.Equal(at) {
		t.Errorf("Get = %+v, want %+v", got, rec)
	}
	if got.CreatedAt.Location() != time.UTC {
		t.Errorf("CreatedAt location = %v, want UTC", got.CreatedAt.Location())
	}
}

func TestSQLiteErrors(t *testing.T) {
	ctx := context.Background()
	s := openMemory(t)
	if _, err := s.Get(ctx, "missing"); !errors.Is(err, ErrConfigNotFound) {
		t.Errorf("Get missing = %v, want ErrConfigNotFound", err)
	}
	rec := Record{ID: "cfg_1", Name: "n", Body: []byte(`{}`), CreatedAt: time.Unix(0, 0)}
	if err := s.Create(ctx, rec); err != nil {
		t.Fatal(err)
	}
	if err := s.Create(ctx, rec); !errors.Is(err, ErrConfigExists) {
		t.Errorf("second Create = %v, want ErrConfigExists", err)
	}
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	if _, err := s.Get(cancelled, "cfg_1"); err == nil || errors.Is(err, ErrConfigNotFound) {
		t.Errorf("Get with a cancelled context = %v, want a context error", err)
	}
}

func TestSQLitePersistsAcrossOpen(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "configs.db")
	s, err := Open(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	if err := s.Create(ctx, Record{ID: "keep", Name: "n", Body: []byte(`{}`), CreatedAt: time.Unix(1, 0)}); err != nil {
		t.Fatal(err)
	}
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}
	s, err = Open(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = s.Close() }()
	if _, err := s.Get(ctx, "keep"); err != nil {
		t.Errorf("Get after reopen = %v", err)
	}
}

func TestSQLiteConcurrentCreates(t *testing.T) {
	ctx := context.Background()
	s, err := Open(ctx, filepath.Join(t.TempDir(), "configs.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = s.Close() }()
	var wg sync.WaitGroup
	errs := make(chan error, 32)
	for i := 0; i < cap(errs); i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			id := fmt.Sprintf("c%d", i)
			if err := s.Create(ctx, Record{ID: id, Name: id, Body: []byte(`{}`), CreatedAt: time.Unix(0, 0)}); err != nil {
				errs <- err
				return
			}
			_, err := s.Get(ctx, id)
			errs <- err
		}(i)
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		if err != nil {
			t.Error(err)
		}
	}
}

func TestOpenRejectsBadPathsAndNewerSchemas(t *testing.T) {
	ctx := context.Background()
	for _, p := range []string{"", "a?b", "a#b"} {
		if _, err := Open(ctx, p); err == nil {
			t.Errorf("Open(%q) succeeded", p)
		}
	}
	path := filepath.Join(t.TempDir(), "future.db")
	db, err := sql.Open("sqlite", "file:"+path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`PRAGMA user_version = 99`); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	if s, err := Open(ctx, path); err == nil {
		_ = s.Close()
		t.Error("Open of a newer schema succeeded")
	}
}

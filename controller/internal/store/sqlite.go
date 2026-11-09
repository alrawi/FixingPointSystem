// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"
	"time"

	"modernc.org/sqlite"
	sqlite3 "modernc.org/sqlite/lib"
)

// MemoryPath is the path that Open treats as a private in memory database.
const MemoryPath = ":memory:"

// schemaVersion is the user_version that the current table layout sets.
const schemaVersion = 2

// migrations[i] holds the statements that move the database from
// user_version i to i+1.
//
// Version 2 adds seq, the creation sequence that listConfigs pages by. An
// INTEGER PRIMARY KEY AUTOINCREMENT never reuses a number and survives VACUUM,
// which the implicit rowid of version 1 does not. The copy keeps the creation
// order of existing rows by reading them in rowid order.
var migrations = [][]string{
	{`CREATE TABLE configs (
		id         TEXT PRIMARY KEY NOT NULL,
		name       TEXT NOT NULL,
		body       BLOB NOT NULL,
		created_at TEXT NOT NULL
	) STRICT`},
	{
		`CREATE TABLE configs_v2 (
			seq        INTEGER PRIMARY KEY AUTOINCREMENT,
			id         TEXT NOT NULL UNIQUE,
			name       TEXT NOT NULL,
			body       BLOB NOT NULL,
			created_at TEXT NOT NULL
		) STRICT`,
		`INSERT INTO configs_v2 (id, name, body, created_at)
			SELECT id, name, body, created_at FROM configs ORDER BY rowid`,
		`DROP TABLE configs`,
		`ALTER TABLE configs_v2 RENAME TO configs`,
	},
}

// SQLite is the Repository on a SQLite database file.
type SQLite struct {
	db *sql.DB
}

var _ Repository = (*SQLite)(nil)

// Open opens or creates the SQLite database at path and brings its tables up
// to date. MemoryPath opens a private database that lives until Close, which
// suits tests. The caller closes the store.
func Open(ctx context.Context, path string) (*SQLite, error) {
	if path == "" || strings.ContainsAny(path, "?#") {
		return nil, fmt.Errorf("open config store %q: path is empty or contains ? or #", path)
	}
	dsn := "file:" + path + "?_pragma=busy_timeout(5000)&_pragma=foreign_keys(1)"
	if path != MemoryPath {
		dsn += "&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)"
	}
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, fmt.Errorf("open config store %s: %w", path, err)
	}
	// SQLite allows one writer at a time, and an in memory database exists
	// once per connection, so the store keeps a single connection.
	db.SetMaxOpenConns(1)
	db.SetConnMaxIdleTime(0)
	db.SetConnMaxLifetime(0)
	s := &SQLite{db: db}
	if err := s.migrate(ctx); err != nil {
		_ = db.Close()
		return nil, fmt.Errorf("open config store %s: %w", path, err)
	}
	return s, nil
}

// migrate applies the migrations that the database lacks in one transaction.
func (s *SQLite) migrate(ctx context.Context) error {
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return fmt.Errorf("begin migration: %w", err)
	}
	defer func() { _ = tx.Rollback() }()
	var version int
	if err := tx.QueryRowContext(ctx, `PRAGMA user_version`).Scan(&version); err != nil {
		return fmt.Errorf("read schema version: %w", err)
	}
	if version > schemaVersion {
		return fmt.Errorf("schema version %d is newer than %d", version, schemaVersion)
	}
	for v := version; v < schemaVersion; v++ {
		for _, stmt := range migrations[v] {
			if _, err := tx.ExecContext(ctx, stmt); err != nil {
				return fmt.Errorf("migrate schema to version %d: %w", v+1, err)
			}
		}
	}
	// PRAGMA takes no bound parameters, and the value is a constant.
	if _, err := tx.ExecContext(ctx, fmt.Sprintf(`PRAGMA user_version = %d`, schemaVersion)); err != nil {
		return fmt.Errorf("write schema version: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("commit migration: %w", err)
	}
	return nil
}

// Close closes the database.
func (s *SQLite) Close() error {
	if err := s.db.Close(); err != nil {
		return fmt.Errorf("close config store: %w", err)
	}
	return nil
}

// Create stores rec.
func (s *SQLite) Create(ctx context.Context, rec Record) error {
	_, err := s.db.ExecContext(ctx,
		`INSERT INTO configs (id, name, body, created_at) VALUES (?, ?, ?, ?)`,
		rec.ID, rec.Name, rec.Body, rec.CreatedAt.UTC().Format(time.RFC3339Nano))
	if err != nil {
		var se *sqlite.Error
		if errors.As(err, &se) && (se.Code() == sqlite3.SQLITE_CONSTRAINT_UNIQUE || se.Code() == sqlite3.SQLITE_CONSTRAINT_PRIMARYKEY) {
			return fmt.Errorf("create config %s: %w", rec.ID, ErrConfigExists)
		}
		return fmt.Errorf("create config %s: %w", rec.ID, err)
	}
	return nil
}

// Get returns the configuration with id.
func (s *SQLite) Get(ctx context.Context, id string) (Record, error) {
	rec := Record{ID: id}
	var created string
	err := s.db.QueryRowContext(ctx,
		`SELECT name, body, created_at FROM configs WHERE id = ?`, id).
		Scan(&rec.Name, &rec.Body, &created)
	if err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return Record{}, fmt.Errorf("get config %s: %w", id, ErrConfigNotFound)
		}
		return Record{}, fmt.Errorf("get config %s: %w", id, err)
	}
	if rec.CreatedAt, err = time.Parse(time.RFC3339Nano, created); err != nil {
		return Record{}, fmt.Errorf("get config %s: created_at: %w", id, err)
	}
	return rec, nil
}

// List returns one page of configuration summaries, newest first.
func (s *SQLite) List(ctx context.Context, opt ListOptions) (Page, error) {
	if opt.Limit < 1 {
		return Page{}, fmt.Errorf("list configs: limit %d is not positive", opt.Limit)
	}
	query := `SELECT seq, id, name, created_at FROM configs`
	var args []any
	if opt.After != 0 {
		query += ` WHERE seq < ?`
		args = append(args, opt.After)
	}
	// One more row than the page holds tells whether a further page exists.
	query += ` ORDER BY seq DESC LIMIT ?`
	args = append(args, opt.Limit+1)
	rows, err := s.db.QueryContext(ctx, query, args...)
	if err != nil {
		return Page{}, fmt.Errorf("list configs: %w", err)
	}
	defer func() { _ = rows.Close() }()
	var page Page
	for rows.Next() {
		var sum Summary
		var created string
		if err := rows.Scan(&sum.Seq, &sum.ID, &sum.Name, &created); err != nil {
			return Page{}, fmt.Errorf("list configs: %w", err)
		}
		if sum.CreatedAt, err = time.Parse(time.RFC3339Nano, created); err != nil {
			return Page{}, fmt.Errorf("list configs: config %s: created_at: %w", sum.ID, err)
		}
		page.Items = append(page.Items, sum)
	}
	if err := rows.Err(); err != nil {
		return Page{}, fmt.Errorf("list configs: %w", err)
	}
	if len(page.Items) > opt.Limit {
		page.Items = page.Items[:opt.Limit]
		page.Next = page.Items[opt.Limit-1].Seq
	}
	return page, nil
}

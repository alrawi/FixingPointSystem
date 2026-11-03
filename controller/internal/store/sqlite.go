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
const schemaVersion = 1

// migrations[i] moves the database from user_version i to i+1.
var migrations = []string{
	`CREATE TABLE configs (
		id         TEXT PRIMARY KEY NOT NULL,
		name       TEXT NOT NULL,
		body       BLOB NOT NULL,
		created_at TEXT NOT NULL
	) STRICT`,
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
		if _, err := tx.ExecContext(ctx, migrations[v]); err != nil {
			return fmt.Errorf("migrate schema to version %d: %w", v+1, err)
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
		if errors.As(err, &se) && se.Code() == sqlite3.SQLITE_CONSTRAINT_PRIMARYKEY {
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

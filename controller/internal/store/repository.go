// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"context"
	"errors"
	"time"
)

// Sentinel errors that callers test with errors.Is.
var (
	// ErrConfigNotFound reports that no stored configuration has the requested id.
	ErrConfigNotFound = errors.New("config not found")
	// ErrConfigExists reports that a configuration with the same id is already stored.
	ErrConfigExists = errors.New("config already exists")
)

// Record is one stored test configuration.
type Record struct {
	// ID is the identifier that the controller assigned, which matches
	// ^[A-Za-z0-9_-]{1,64}$.
	ID string
	// Name is metadata.name of the configuration.
	Name string
	// Body is the configuration as canonical JSON, with sorted object keys.
	Body []byte
	// CreatedAt is the time the controller stored the configuration, in UTC.
	CreatedAt time.Time
}

// Summary is one stored configuration without its body, as listConfigs
// reports it.
type Summary struct {
	// Seq is the creation sequence. It grows with every stored configuration
	// and is never reused, so it orders configurations by creation and anchors
	// the page tokens.
	Seq uint64
	// ID is the identifier of the configuration.
	ID string
	// Name is metadata.name of the configuration.
	Name string
	// CreatedAt is the time the controller stored the configuration, in UTC.
	CreatedAt time.Time
}

// ListOptions selects one page of List.
type ListOptions struct {
	// Limit is the page size and must be positive.
	Limit int
	// After, when not zero, returns only configurations with a lower Seq, so a
	// page continues below the last item of the page before it.
	After uint64
}

// Page is one page of summaries, newest first.
type Page struct {
	// Items holds at most Limit summaries.
	Items []Summary
	// Next is the Seq of the last item when a further page exists, and zero
	// when this page reaches the oldest configuration.
	Next uint64
}

// Repository stores test configurations. Implementations are safe for
// concurrent use.
type Repository interface {
	// Create stores rec. It fails with ErrConfigExists when rec.ID is taken.
	Create(ctx context.Context, rec Record) error
	// Get returns the configuration with id. It fails with ErrConfigNotFound
	// when there is none.
	Get(ctx context.Context, id string) (Record, error)
	// List returns configurations newest first by creation sequence. A
	// configuration stored between two calls has a higher sequence, so it
	// never shifts or repeats an item of a later page.
	List(ctx context.Context, opt ListOptions) (Page, error)
}

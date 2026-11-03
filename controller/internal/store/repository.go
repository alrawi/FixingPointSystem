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

// Repository stores test configurations. Implementations are safe for
// concurrent use.
type Repository interface {
	// Create stores rec. It fails with ErrConfigExists when rec.ID is taken.
	Create(ctx context.Context, rec Record) error
	// Get returns the configuration with id. It fails with ErrConfigNotFound
	// when there is none.
	Get(ctx context.Context, id string) (Record, error)
}

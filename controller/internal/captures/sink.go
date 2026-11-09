// SPDX-License-Identifier: GPL-3.0-only

package captures

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"
	"sync"
)

// errSinkClosed reports a write after the download gave up on the transfer.
var errSinkClosed = errors.New("capture sink closed")

// fileSink stores the chunks of one upload attempt in a part file. The file
// holds the bytes of earlier attempts below the start offset, so a resumed
// attempt appends and the digest covers the bytes of all attempts.
type fileSink struct {
	path   string
	size   uint64 // size that the agent reported at the end of the capture
	digest string // digest that the agent reported at the end of the capture

	mu     sync.Mutex
	f      *os.File
	held   uint64 // bytes in the file when the attempt began
	next   uint64 // offset of the next byte
	closed bool
}

func newSink(path string, held, size uint64, digest string) *fileSink {
	return &fileSink{path: path, size: size, digest: digest, held: held, next: held}
}

// WriteChunk implements agents.CaptureSink.
func (s *fileSink) WriteChunk(offset uint64, data []byte) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed {
		return errSinkClosed
	}
	if offset != s.next || offset+uint64(len(data)) > s.size {
		return fmt.Errorf("chunk of %d bytes at offset %d, expected offset %d of a file of %d bytes", len(data), offset, s.next, s.size)
	}
	if s.f == nil {
		f, err := os.OpenFile(s.path, os.O_WRONLY|os.O_CREATE, 0o600)
		if err != nil {
			return err
		}
		// Bytes beyond the held part cannot be trusted.
		if err := f.Truncate(int64(s.held)); err != nil { //nolint:gosec // G115: bounded by MaxFileBytes
			_ = f.Close()
			return err
		}
		s.f = f
	}
	if _, err := s.f.WriteAt(data, int64(offset)); err != nil { //nolint:gosec // G115: bounded by MaxFileBytes
		return err
	}
	s.next += uint64(len(data))
	return nil
}

// End implements agents.CaptureSink. It checks the size and the SHA-256 of the
// whole file, the bytes of every attempt together, against what the agent sent
// in the end message and what it reported when the capture ended. A file that
// fails the check is removed, so the next attempt starts at offset zero.
func (s *fileSink) End(total uint64, digest string) (uint64, bool, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed {
		return 0, false, errSinkClosed
	}
	if s.f != nil {
		if err := s.f.Sync(); err != nil {
			return s.next, false, err
		}
		_ = s.f.Close()
		s.f = nil
	}
	s.closed = true
	got, err := fileDigest(s.path)
	if err != nil {
		return s.next, false, err
	}
	if total != s.size || s.next != s.size || digest != s.digest || got != s.digest {
		_ = os.Remove(s.path)
		return 0, false, fmt.Errorf("%w: %d bytes with digest %s received, end message says %d bytes and %q, the capture ended with %d bytes and %q",
			ErrDigestMismatch, s.next, got, total, digest, s.size, s.digest)
	}
	return s.next, true, nil
}

// close releases the file. Later writes fail, so an upload that outlives its
// attempt cannot write into the next one.
func (s *fileSink) close() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.closed = true
	if s.f != nil {
		_ = s.f.Close()
		s.f = nil
	}
}

// fileSize returns the size of the file at path, or zero when it does not
// exist.
func fileSize(path string) uint64 {
	fi, err := os.Stat(path)
	if err != nil || fi.Size() < 0 {
		return 0
	}
	return uint64(fi.Size()) //nolint:gosec // G115: checked non negative above
}

// fileDigest returns sha256: and the lowercase hex SHA-256 of the file at path.
// A file that does not exist has the digest of no bytes.
func fileDigest(path string) (string, error) {
	h := sha256.New()
	f, err := os.Open(path) //nolint:gosec // G304: the path is built from validated identities by the service
	switch {
	case errors.Is(err, os.ErrNotExist):
	case err != nil:
		return "", err
	default:
		defer func() { _ = f.Close() }()
		if _, err := io.Copy(h, f); err != nil {
			return "", fmt.Errorf("hash %s: %w", path, err)
		}
	}
	return "sha256:" + hex.EncodeToString(h.Sum(nil)), nil
}

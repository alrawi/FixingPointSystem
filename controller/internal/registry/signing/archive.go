// SPDX-License-Identifier: GPL-3.0-only

package signing

import (
	"archive/tar"
	"bufio"
	"compress/gzip"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"regexp"
	"sort"
	"strings"
)

// Limits bound what ArchiveDigest reads. They have the meaning of
// registry.Limits.
type Limits struct {
	// MaxBytes bounds the sum of the file sizes after decompression.
	MaxBytes int64
	// MaxFiles bounds the number of regular files.
	MaxFiles int
}

// DefaultLimits match registry.DefaultLimits.
var DefaultLimits = Limits{MaxBytes: 256 << 20, MaxFiles: 10000}

// entryPath is the relpath pattern of the manifest schema, the same rule that
// registry install applies to every archive entry.
var entryPath = regexp.MustCompile(`^[A-Za-z0-9_.\-]+(/[A-Za-z0-9_.\-]+)*$`)

// ArchiveDigest returns the module tree digest of the tar archive in r, which
// may be gzip compressed, without writing anything to disk. It applies rule 1
// of contracts/README.md to the entries, so for an archive of a tree it equals
// registry.TreeDigest of that tree, and it applies the entry rules of registry
// install: only directories and regular files, relative paths of relpath
// characters with no "." or ".." segment, and no duplicates. A zero lim selects
// DefaultLimits.
//
// The digest covers file names and contents only. Modes, owners, times, and
// empty directories do not count, as in registry.TreeDigest, so they are not
// protected by a signature.
func ArchiveDigest(r io.Reader, lim Limits) (string, error) {
	if lim == (Limits{}) {
		lim = DefaultLimits
	}
	br := bufio.NewReader(r)
	head, err := br.Peek(2)
	if err != nil && !errors.Is(err, io.EOF) {
		return "", fmt.Errorf("read archive: %w", err)
	}
	var src io.Reader = br
	if len(head) == 2 && head[0] == 0x1f && head[1] == 0x8b {
		zr, err := gzip.NewReader(br)
		if err != nil {
			return "", fmt.Errorf("%w: gzip header: %s", ErrArchive, err.Error())
		}
		defer func() { _ = zr.Close() }()
		src = zr
	}
	type line struct{ path, text string }
	var lines []line
	tr := tar.NewReader(src)
	var total int64
	seen := map[string]bool{}
	for {
		h, err := tr.Next()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return "", fmt.Errorf("%w: %s", ErrArchive, err.Error())
		}
		name := strings.TrimSuffix(strings.TrimPrefix(h.Name, "./"), "/")
		if name == "" || name == "." {
			continue
		}
		if !entryPath.MatchString(name) || hasDotSegment(name) {
			return "", fmt.Errorf("%w: entry %q has a path that a manifest cannot name", ErrArchive, h.Name)
		}
		if seen[name] {
			return "", fmt.Errorf("%w: entry %q appears twice", ErrArchive, name)
		}
		seen[name] = true
		switch h.Typeflag {
		case tar.TypeDir:
		case tar.TypeReg:
			total += h.Size
			if len(lines)+1 > lim.MaxFiles {
				return "", fmt.Errorf("%w: more than %d files", ErrArchiveTooLarge, lim.MaxFiles)
			}
			if h.Size < 0 || total > lim.MaxBytes {
				return "", fmt.Errorf("%w: more than %d bytes", ErrArchiveTooLarge, lim.MaxBytes)
			}
			hash := sha256.New()
			n, err := io.Copy(hash, io.LimitReader(tr, h.Size))
			if err != nil {
				return "", fmt.Errorf("%w: read %s: %s", ErrArchive, name, err.Error())
			}
			if n != h.Size {
				return "", fmt.Errorf("%w: entry %q is truncated", ErrArchive, name)
			}
			lines = append(lines, line{name, hex.EncodeToString(hash.Sum(nil)) + "  " + name + "\n"})
		default:
			return "", fmt.Errorf("%w: entry %q is not a regular file or directory", ErrArchive, name)
		}
	}
	sort.Slice(lines, func(i, j int) bool { return lines[i].path < lines[j].path })
	var b strings.Builder
	for _, l := range lines {
		b.WriteString(l.text)
	}
	sum := sha256.Sum256([]byte(b.String()))
	return "sha256:" + hex.EncodeToString(sum[:]), nil
}

func hasDotSegment(p string) bool {
	for _, s := range strings.Split(p, "/") {
		if s == "." || s == ".." {
			return true
		}
	}
	return false
}

// SignArchive digests the archive in r and signs the digest. It returns the
// digest too, so a publisher can record it in the index.
func (s *Signer) SignArchive(r io.Reader, lim Limits) (digest string, sig Signature, err error) {
	digest, err = ArchiveDigest(r, lim)
	if err != nil {
		return "", Signature{}, fmt.Errorf("sign archive: %w", err)
	}
	sig, err = s.SignDigest(digest)
	if err != nil {
		return "", Signature{}, err
	}
	return digest, sig, nil
}

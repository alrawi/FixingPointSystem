// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"archive/tar"
	"bufio"
	"compress/gzip"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

// ErrArchive reports a module archive that is not a tar archive, or that holds
// an entry the registry refuses, such as an absolute path or a link.
var ErrArchive = errors.New("invalid module archive")

// ErrArchiveTooLarge reports an archive that exceeds the registry limits.
var ErrArchiveTooLarge = errors.New("module archive is too large")

// Limits bound what one module archive may hold.
type Limits struct {
	// MaxBytes bounds the sum of the file sizes after decompression.
	MaxBytes int64
	// MaxFiles bounds the number of regular files.
	MaxFiles int
}

// DefaultLimits are the limits that New uses when the caller passes none.
var DefaultLimits = Limits{MaxBytes: 256 << 20, MaxFiles: 10000}

// entryPath accepts the relative paths that the manifest schema calls relpath,
// so every file of an installed module can be named in a manifest.
var entryPath = regexp.MustCompile(`^[A-Za-z0-9_.\-]+(/[A-Za-z0-9_.\-]+)*$`)

// unpack writes the tar archive in r, which may be gzip compressed, into the
// empty directory dst. The archive holds the module tree with manifest.yaml at
// its root. Only directories and regular files are allowed, and every path is
// relative, uses only the relpath characters, and has no "." or ".." segment.
func unpack(r io.Reader, dst string, lim Limits) error {
	br := bufio.NewReader(r)
	head, err := br.Peek(2)
	if err != nil && !errors.Is(err, io.EOF) {
		return fmt.Errorf("read archive: %w", err)
	}
	var src io.Reader = br
	if len(head) == 2 && head[0] == 0x1f && head[1] == 0x8b {
		zr, err := gzip.NewReader(br)
		if err != nil {
			return fmt.Errorf("%w: gzip header: %s", ErrArchive, err.Error())
		}
		defer func() { _ = zr.Close() }()
		src = zr
	}
	tr := tar.NewReader(src)
	var total int64
	files := 0
	seen := map[string]bool{}
	for {
		h, err := tr.Next()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return fmt.Errorf("%w: %s", ErrArchive, err.Error())
		}
		name := strings.TrimPrefix(h.Name, "./")
		name = strings.TrimSuffix(name, "/")
		if name == "" || name == "." {
			continue
		}
		if !entryPath.MatchString(name) || hasDotSegment(name) {
			return fmt.Errorf("%w: entry %q has a path that a manifest cannot name", ErrArchive, h.Name)
		}
		if seen[name] {
			return fmt.Errorf("%w: entry %q appears twice", ErrArchive, name)
		}
		seen[name] = true
		target := filepath.Join(dst, filepath.FromSlash(name))
		switch h.Typeflag {
		case tar.TypeDir:
			if err := os.MkdirAll(target, 0o750); err != nil {
				return fmt.Errorf("unpack %s: %w", name, err)
			}
		case tar.TypeReg:
			files++
			total += h.Size
			if files > lim.MaxFiles {
				return fmt.Errorf("%w: more than %d files", ErrArchiveTooLarge, lim.MaxFiles)
			}
			if h.Size < 0 || total > lim.MaxBytes {
				return fmt.Errorf("%w: more than %d bytes", ErrArchiveTooLarge, lim.MaxBytes)
			}
			if err := writeEntry(tr, target, h.Size); err != nil {
				return fmt.Errorf("unpack %s: %w", name, err)
			}
		default:
			return fmt.Errorf("%w: entry %q is not a regular file or directory", ErrArchive, name)
		}
	}
	if !seen[ManifestFile] {
		return fmt.Errorf("%w: no %s at the archive root", ErrArchive, ManifestFile)
	}
	return nil
}

func hasDotSegment(p string) bool {
	for _, s := range strings.Split(p, "/") {
		if s == "." || s == ".." {
			return true
		}
	}
	return false
}

func writeEntry(r io.Reader, target string, size int64) error {
	if err := os.MkdirAll(filepath.Dir(target), 0o750); err != nil {
		return err
	}
	f, err := os.OpenFile(filepath.Clean(target), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return err
	}
	n, err := io.Copy(f, io.LimitReader(r, size))
	if cerr := f.Close(); err == nil {
		err = cerr
	}
	if err == nil && n != size {
		err = fmt.Errorf("%w: entry is truncated", ErrArchive)
	}
	return err
}

// copyTree copies the module tree at src into the empty directory dst with the
// same rules as unpack, so a directory install and an archive install accept
// the same trees. Reads go through an os.Root, so a link cannot lead outside
// src.
func copyTree(src, dst string, lim Limits) error {
	root, err := os.OpenRoot(src)
	if err != nil {
		return err
	}
	defer func() { _ = root.Close() }()
	fsys := root.FS()
	var total int64
	files := 0
	return fs.WalkDir(fsys, ".", func(name string, d fs.DirEntry, err error) error {
		switch {
		case err != nil:
			return err
		case name == ".":
			return nil
		case !entryPath.MatchString(name) || hasDotSegment(name):
			return fmt.Errorf("%w: file %q has a path that a manifest cannot name", ErrArchive, name)
		}
		target := filepath.Join(dst, filepath.FromSlash(name))
		switch {
		case d.IsDir():
			return os.MkdirAll(target, 0o750)
		case !d.Type().IsRegular():
			return fmt.Errorf("%s: %w", name, ErrSymlink)
		}
		info, err := d.Info()
		if err != nil {
			return err
		}
		files++
		total += info.Size()
		if files > lim.MaxFiles || total > lim.MaxBytes {
			return fmt.Errorf("%w: limits are %d files and %d bytes", ErrArchiveTooLarge, lim.MaxFiles, lim.MaxBytes)
		}
		in, err := fsys.Open(name)
		if err != nil {
			return err
		}
		defer func() { _ = in.Close() }()
		return writeEntry(in, target, info.Size())
	})
}

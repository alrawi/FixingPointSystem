// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"sort"
	"strings"
)

// ErrSymlink reports a symbolic link or another non regular file in a module
// tree, which the digest rule of contracts/README.md does not allow.
var ErrSymlink = errors.New("module tree holds a symbolic link or special file")

// DigestPrefix starts every module digest.
const DigestPrefix = "sha256:"

// TreeDigest returns the module tree digest of the directory dir, as plan rule
// 1 of contracts/README.md defines it. Every regular file contributes the line
// "<sha256 hex of the file>  <relative POSIX path>\n", the lines sort by the
// bytes of the path, and the digest is "sha256:" and the hex SHA-256 of their
// concatenation. A symbolic link anywhere in the tree is an error.
func TreeDigest(dir string) (string, error) {
	type line struct{ path, text string }
	var lines []line
	root, err := os.OpenRoot(dir)
	if err != nil {
		return "", fmt.Errorf("digest module tree %s: %w", dir, err)
	}
	defer func() { _ = root.Close() }()
	fsys := root.FS()
	err = fs.WalkDir(fsys, ".", func(rel string, d fs.DirEntry, err error) error {
		switch {
		case err != nil:
			return err
		case d.IsDir():
			return nil
		case !d.Type().IsRegular():
			return fmt.Errorf("%s: %w", rel, ErrSymlink)
		}
		sum, err := fileSHA256(fsys, rel)
		if err != nil {
			return err
		}
		lines = append(lines, line{rel, sum + "  " + rel + "\n"})
		return nil
	})
	if err != nil {
		return "", fmt.Errorf("digest module tree %s: %w", dir, err)
	}
	sort.Slice(lines, func(i, j int) bool { return lines[i].path < lines[j].path })
	var b strings.Builder
	for _, l := range lines {
		b.WriteString(l.text)
	}
	sum := sha256.Sum256([]byte(b.String()))
	return DigestPrefix + hex.EncodeToString(sum[:]), nil
}

func fileSHA256(fsys fs.FS, p string) (string, error) {
	f, err := fsys.Open(p)
	if err != nil {
		return "", err
	}
	defer func() { _ = f.Close() }()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", fmt.Errorf("read %s: %w", p, err)
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}

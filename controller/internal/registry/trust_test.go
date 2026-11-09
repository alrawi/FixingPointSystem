// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"encoding/base64"
	"errors"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/registry/signing"
)

// seedSigner returns the deterministic signer of the CTL-11 golden vectors for
// seed 0x07, whose key id is ed25519:fe812c12f3ab4ce6ac5db69ac352f906, or the
// signer of another seed.
func seedSigner(t *testing.T, seed byte) *signing.Signer {
	t.Helper()
	s, err := signing.NewSigner(ed25519.NewKeyFromSeed(bytes.Repeat([]byte{seed}, ed25519.SeedSize)))
	if err != nil {
		t.Fatal(err)
	}
	return s
}

const goldenKeyID = "ed25519:fe812c12f3ab4ce6ac5db69ac352f906"

func trustOf(t *testing.T, signers ...*signing.Signer) *signing.TrustStore {
	t.Helper()
	ts := signing.NewTrustStore()
	for _, s := range signers {
		if _, err := ts.Add(s.PublicKey()); err != nil {
			t.Fatal(err)
		}
	}
	return ts
}

func openWith(t *testing.T, p TrustPolicy) *Registry {
	t.Helper()
	r, err := Open(filepath.Join(t.TempDir(), "registry"), Limits{}, WithTrust(p))
	if err != nil {
		t.Fatal(err)
	}
	return r
}

// tampered returns the archive of the example module with a comment appended
// to its manifest, so the tree digest differs from the original.
func tampered(t *testing.T, name string, gz bool) []byte {
	t.Helper()
	return tarFiles(t, exampleTree(t, name, func(s string) string { return s + "\n# tampered\n" }), gz)
}

func signedBy(t *testing.T, s *signing.Signer, archive []byte) signing.Signature {
	t.Helper()
	_, sig, err := s.SignArchive(bytes.NewReader(archive), signing.Limits{})
	if err != nil {
		t.Fatal(err)
	}
	return sig
}

func requireNothingInstalled(t *testing.T, r *Registry) {
	t.Helper()
	if got := r.List(context.Background(), ""); len(got) != 0 {
		t.Fatalf("registry holds %v, want nothing", got)
	}
	if _, err := os.Stat(r.modulesDir()); err == nil {
		entries, _ := os.ReadDir(r.modulesDir())
		if len(entries) != 0 {
			t.Fatalf("modules directory holds %d entries after a refused install", len(entries))
		}
	}
	staged, _ := os.ReadDir(r.stagingDir())
	if len(staged) != 0 {
		t.Fatalf("staging directory holds %d entries after a refused install", len(staged))
	}
}

func TestInstallAcceptsASignedModule(t *testing.T) {
	ctx := context.Background()
	s := seedSigner(t, 7)
	if s.KeyID() != goldenKeyID {
		t.Fatalf("golden key id = %s", s.KeyID())
	}
	for _, gz := range []bool{false, true} {
		r := openWith(t, TrustPolicy{Store: trustOf(t, s)})
		archive := tarFiles(t, exampleTree(t, "dns", nil), gz)
		sig := signedBy(t, s, archive)
		e, err := r.Install(ctx, bytes.NewReader(archive), WithSignature(sig))
		if err != nil {
			t.Fatalf("Install(signed, gzip=%v) = %v", gz, err)
		}
		if e.SignatureKeyID != goldenKeyID || e.Signature != base64.StdEncoding.EncodeToString(sig.Bytes) {
			t.Fatalf("entry carries signature %q key %q", e.Signature, e.SignatureKeyID)
		}
		got, ok := e.StoredSignature()
		if !ok || got.KeyID != sig.KeyID || !bytes.Equal(got.Bytes, sig.Bytes) {
			t.Fatalf("StoredSignature = %v, %v", got, ok)
		}
		// The stored signature verifies the digest that plans carry.
		if err := signing.NewVerifier(trustOf(t, s), nil).VerifyDigest(e.Digest, got); err != nil {
			t.Fatalf("stored signature does not verify the entry digest: %v", err)
		}
		// The signature survives a reopen.
		r2, err := Open(r.Root(), Limits{}, WithTrust(TrustPolicy{Store: trustOf(t, s)}))
		if err != nil {
			t.Fatal(err)
		}
		e2, err := r2.Entry(ctx, e.Name, e.Version)
		if err != nil {
			t.Fatal(err)
		}
		if e2.Signature != e.Signature || e2.SignatureKeyID != e.SignatureKeyID {
			t.Fatalf("reopened entry carries signature %q key %q", e2.Signature, e2.SignatureKeyID)
		}
		if err := r2.Verify(ctx, e.Name, e.Version); err != nil {
			t.Fatal(err)
		}
	}
}

func TestInstallRefusesAModifiedArchive(t *testing.T) {
	s := seedSigner(t, 7)
	r := openWith(t, TrustPolicy{Store: trustOf(t, s)})
	original := tarFiles(t, exampleTree(t, "dns", nil), false)
	sig := signedBy(t, s, original)
	// Even with the development setting, a signature that does not match
	// is never accepted.
	for _, allow := range []bool{false, true} {
		r.trust.AllowUnsigned = allow
		_, err := r.Install(context.Background(), bytes.NewReader(tampered(t, "dns", false)), WithSignature(sig))
		if !errors.Is(err, ErrUntrusted) || !errors.Is(err, signing.ErrBadSignature) {
			t.Fatalf("Install(modified archive, allow_unsigned=%v) = %v, want ErrUntrusted and ErrBadSignature", allow, err)
		}
	}
	requireNothingInstalled(t, r)
}

func TestInstallRefusesKeysThatTheStoreDoesNotTrust(t *testing.T) {
	good, stranger, revoked, retired := seedSigner(t, 7), seedSigner(t, 8), seedSigner(t, 9), seedSigner(t, 10)
	ts := trustOf(t, good, revoked, retired)
	if err := ts.Revoke(revoked.KeyID()); err != nil {
		t.Fatal(err)
	}
	past := time.Now().Add(-time.Hour)
	if err := ts.Retire(retired.KeyID(), past); err != nil {
		t.Fatal(err)
	}
	r := openWith(t, TrustPolicy{Store: ts})
	archive := tarFiles(t, exampleTree(t, "dns", nil), false)
	for _, tt := range []struct {
		name   string
		signer *signing.Signer
		want   error
	}{
		{"unknown key", stranger, signing.ErrUnknownKey},
		{"revoked key", revoked, signing.ErrKeyRevoked},
		{"retired key past its cutoff", retired, signing.ErrKeyExpired},
	} {
		_, err := r.Install(context.Background(), bytes.NewReader(archive), WithSignature(signedBy(t, tt.signer, archive)))
		if !errors.Is(err, ErrUntrusted) || !errors.Is(err, tt.want) {
			t.Errorf("%s: Install = %v, want ErrUntrusted and %v", tt.name, err, tt.want)
		}
	}
	// A signature of the wrong length is refused too.
	sig := signedBy(t, good, archive)
	sig.Bytes = sig.Bytes[:10]
	if _, err := r.Install(context.Background(), bytes.NewReader(archive), WithSignature(sig)); !errors.Is(err, ErrUntrusted) || !errors.Is(err, signing.ErrSignatureLength) {
		t.Errorf("truncated signature: Install = %v", err)
	}
	requireNothingInstalled(t, r)
}

func TestInstallRefusesAnUnsignedModuleUnlessDevelopmentAllowsIt(t *testing.T) {
	ctx := context.Background()
	s := seedSigner(t, 7)
	archive := tarFiles(t, exampleTree(t, "dns", nil), false)

	r := openWith(t, TrustPolicy{Store: trustOf(t, s)})
	_, err := r.Install(ctx, bytes.NewReader(archive))
	if !errors.Is(err, ErrUntrusted) || !errors.Is(err, signing.ErrNoSignature) {
		t.Fatalf("Install(unsigned) = %v, want ErrUntrusted and ErrNoSignature", err)
	}
	if _, err := r.InstallDir(ctx, filepath.Join(examplesDir, "dns")); !errors.Is(err, ErrUntrusted) {
		t.Fatalf("InstallDir(unsigned) = %v, want ErrUntrusted", err)
	}
	requireNothingInstalled(t, r)

	var logs strings.Builder
	dev, err := Open(filepath.Join(t.TempDir(), "dev"), Limits{},
		WithTrust(TrustPolicy{Store: trustOf(t, s), AllowUnsigned: true}),
		WithLogger(slog.New(slog.NewTextHandler(&logs, nil))))
	if err != nil {
		t.Fatal(err)
	}
	e, err := dev.Install(ctx, bytes.NewReader(archive))
	if err != nil {
		t.Fatalf("Install(unsigned, allow_unsigned) = %v", err)
	}
	if e.Signature != "" || e.SignatureKeyID != "" {
		t.Fatalf("an unsigned install stored signature %q key %q", e.Signature, e.SignatureKeyID)
	}
	if _, ok := e.StoredSignature(); ok {
		t.Fatal("StoredSignature reports a signature for an unsigned module")
	}
	if !strings.Contains(logs.String(), "without a signature") {
		t.Fatalf("no warning about the unsigned install in %q", logs.String())
	}
}

func TestInstallWithoutATrustStoreDoesNotRequireSignatures(t *testing.T) {
	ctx := context.Background()
	var logs strings.Builder
	r, err := Open(filepath.Join(t.TempDir(), "registry"), Limits{}, WithLogger(slog.New(slog.NewTextHandler(&logs, nil))))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(logs.String(), "no trust store") {
		t.Fatalf("Open logged %q, want a warning about the missing trust store", logs.String())
	}
	e, err := r.Install(ctx, bytes.NewReader(tarFiles(t, exampleTree(t, "dns", nil), false)))
	if err != nil {
		t.Fatalf("Install(unsigned, no trust store) = %v", err)
	}
	if e.Signature != "" {
		t.Fatalf("entry carries signature %q", e.Signature)
	}
	// A signature that cannot be checked is not stored, because a stored
	// signature means that the registry verified it.
	s := seedSigner(t, 7)
	archive := tarFiles(t, exampleTree(t, "http1", nil), false)
	e, err = r.Install(ctx, bytes.NewReader(archive), WithSignature(signedBy(t, s, archive)))
	if err != nil {
		t.Fatal(err)
	}
	if e.Signature != "" || e.SignatureKeyID != "" {
		t.Fatalf("an unchecked signature was stored: %q %q", e.Signature, e.SignatureKeyID)
	}
	// The example modules install from their directories without a signature.
	fresh, err := Open(filepath.Join(t.TempDir(), "fresh"), Limits{})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := fresh.InstallDir(ctx, filepath.Join(examplesDir, "dns")); err != nil {
		t.Fatalf("InstallDir(example module, no trust store) = %v", err)
	}
}

func TestInstallDirVerifiesTheTree(t *testing.T) {
	ctx := context.Background()
	s := seedSigner(t, 7)
	dir := filepath.Join(examplesDir, "dns")
	digest, err := TreeDigest(dir)
	if err != nil {
		t.Fatal(err)
	}
	good, err := s.SignDigest(digest)
	if err != nil {
		t.Fatal(err)
	}
	r := openWith(t, TrustPolicy{Store: trustOf(t, s)})
	other, err := s.SignDigest("sha256:" + strings.Repeat("0", 64))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := r.InstallDir(ctx, dir, WithSignature(other)); !errors.Is(err, ErrUntrusted) || !errors.Is(err, signing.ErrBadSignature) {
		t.Fatalf("InstallDir(wrong signature) = %v", err)
	}
	requireNothingInstalled(t, r)
	e, err := r.InstallDir(ctx, dir, WithSignature(good))
	if err != nil {
		t.Fatalf("InstallDir(signed) = %v", err)
	}
	if e.SignatureKeyID != goldenKeyID {
		t.Fatalf("entry key id = %q", e.SignatureKeyID)
	}
}

func TestTrustStoreFile(t *testing.T) {
	ctx := context.Background()
	s := seedSigner(t, 7)
	path := filepath.Join(t.TempDir(), "trust.json")
	write := func(ts *signing.TrustStore) {
		t.Helper()
		data, err := ts.Encode()
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, data, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	ts := trustOf(t, s)
	write(ts)
	r := openWith(t, TrustPolicy{StorePath: path})
	archive := tarFiles(t, exampleTree(t, "dns", nil), false)
	sig := signedBy(t, s, archive)
	if _, err := r.Install(ctx, bytes.NewReader(archive), WithSignature(sig)); err != nil {
		t.Fatalf("Install with the key in the file = %v", err)
	}
	// Revoking the key in the file takes effect without a restart.
	if err := ts.Revoke(s.KeyID()); err != nil {
		t.Fatal(err)
	}
	write(ts)
	archive2 := tarFiles(t, exampleTree(t, "http1", nil), false)
	if _, err := r.Install(ctx, bytes.NewReader(archive2), WithSignature(signedBy(t, s, archive2))); !errors.Is(err, signing.ErrKeyRevoked) {
		t.Fatalf("Install after revocation = %v, want ErrKeyRevoked", err)
	}
	// An unreadable or malformed file stops the registry from opening, and
	// an install fails closed when the file goes bad later.
	if _, err := Open(filepath.Join(t.TempDir(), "r"), Limits{}, WithTrust(TrustPolicy{StorePath: filepath.Join(t.TempDir(), "missing.json")})); err == nil {
		t.Fatal("Open accepted a missing trust store file")
	}
	if err := os.WriteFile(path, []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := Open(filepath.Join(t.TempDir(), "r2"), Limits{}, WithTrust(TrustPolicy{StorePath: path})); !errors.Is(err, signing.ErrTrustStore) {
		t.Fatalf("Open(malformed trust store) = %v, want ErrTrustStore", err)
	}
	if _, err := r.Install(ctx, bytes.NewReader(archive2), WithSignature(signedBy(t, s, archive2))); err == nil || errors.Is(err, ErrUntrusted) {
		t.Fatalf("Install with a broken trust store file = %v, want a plain failure", err)
	}
}

func TestSpoolRefusesAnEndlessStream(t *testing.T) {
	r, err := Open(filepath.Join(t.TempDir(), "small"), Limits{MaxBytes: 1024, MaxFiles: 2}, WithTrust(TrustPolicy{Store: trustOf(t, seedSigner(t, 7))}))
	if err != nil {
		t.Fatal(err)
	}
	sig := signing.Signature{KeyID: goldenKeyID, Bytes: make([]byte, signing.SignatureSize)}
	_, err = r.Install(context.Background(), zeroReader{}, WithSignature(sig))
	if !errors.Is(err, ErrArchiveTooLarge) {
		t.Fatalf("Install(endless stream) = %v, want ErrArchiveTooLarge", err)
	}
	requireNothingInstalled(t, r)
}

type zeroReader struct{}

func (zeroReader) Read(p []byte) (int, error) { clear(p); return len(p), nil }

func TestInstallModuleOverHTTPChecksSignatures(t *testing.T) {
	s := seedSigner(t, 7)
	r := openWith(t, TrustPolicy{Store: trustOf(t, s)})
	h := newAPIHarness(t, r).h
	archive := tarFiles(t, exampleTree(t, "dns", nil), false)
	sig := signedBy(t, s, archive)
	post := func(body []byte, headers map[string]string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(http.MethodPost, api.BasePath+"/modules", bytes.NewReader(body))
		req.Header.Set("Authorization", "Bearer "+adminToken)
		req.Header.Set("Content-Type", "application/octet-stream")
		for k, v := range headers {
			req.Header.Set(k, v)
		}
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		return rec
	}
	enc := base64.StdEncoding.EncodeToString(sig.Bytes)
	good := map[string]string{SignatureKeyHeader: sig.KeyID, SignatureHeader: enc}
	for _, tt := range []struct {
		name    string
		body    []byte
		headers map[string]string
		want    int
	}{
		{"modified archive", tampered(t, "dns", false), good, http.StatusForbidden},
		{"unsigned", archive, nil, http.StatusForbidden},
		{"unknown key", archive, map[string]string{SignatureKeyHeader: "ed25519:00000000000000000000000000000000", SignatureHeader: enc}, http.StatusForbidden},
		{"key id without signature", archive, map[string]string{SignatureKeyHeader: sig.KeyID}, http.StatusBadRequest},
		{"signature that is not base64", archive, map[string]string{SignatureKeyHeader: sig.KeyID, SignatureHeader: "!!"}, http.StatusBadRequest},
		{"signed", archive, good, http.StatusCreated},
	} {
		if rec := post(tt.body, tt.headers); rec.Code != tt.want {
			t.Errorf("%s: status %d, want %d, body %s", tt.name, rec.Code, tt.want, rec.Body.String())
		}
	}
	if n := len(r.List(context.Background(), "")); n != 1 {
		t.Fatalf("registry holds %d modules, want the one signed module", n)
	}
}

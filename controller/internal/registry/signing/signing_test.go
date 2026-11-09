// SPDX-License-Identifier: GPL-3.0-only

package signing_test

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"crypto/ed25519"
	"crypto/rand"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/registry/signing"
)

var epoch = time.Date(2026, 11, 8, 12, 0, 0, 0, time.UTC)

// signer returns a deterministic signer, so failures reproduce.
func signer(t *testing.T, seed byte) *signing.Signer {
	t.Helper()
	s, err := signing.NewSigner(ed25519.NewKeyFromSeed(bytes.Repeat([]byte{seed}, ed25519.SeedSize)))
	if err != nil {
		t.Fatal(err)
	}
	return s
}

// trust returns a store that trusts the given signers as active.
func trust(t *testing.T, signers ...*signing.Signer) *signing.TrustStore {
	t.Helper()
	ts := signing.NewTrustStore()
	for _, s := range signers {
		if _, err := ts.Add(s.PublicKey()); err != nil {
			t.Fatal(err)
		}
	}
	return ts
}

func verifier(ts *signing.TrustStore, now time.Time) *signing.Verifier {
	return signing.NewVerifier(ts, func() time.Time { return now })
}

type file struct{ name, body string }

// archive writes a tar archive of the files, gzip compressed when gz is true.
func archive(t *testing.T, gz bool, files ...file) []byte {
	t.Helper()
	var buf bytes.Buffer
	var w interface {
		Write([]byte) (int, error)
	} = &buf
	var zw *gzip.Writer
	if gz {
		zw = gzip.NewWriter(&buf)
		w = zw
	}
	tw := tar.NewWriter(w)
	for _, f := range files {
		h := &tar.Header{Name: f.name, Typeflag: tar.TypeReg, Mode: 0o644, Size: int64(len(f.body))}
		if strings.HasSuffix(f.name, "/") {
			h = &tar.Header{Name: f.name, Typeflag: tar.TypeDir, Mode: 0o755}
		}
		if err := tw.WriteHeader(h); err != nil {
			t.Fatal(err)
		}
		if _, err := tw.Write([]byte(f.body)); err != nil && h.Typeflag == tar.TypeReg {
			t.Fatal(err)
		}
	}
	if err := tw.Close(); err != nil {
		t.Fatal(err)
	}
	if zw != nil {
		if err := zw.Close(); err != nil {
			t.Fatal(err)
		}
	}
	return buf.Bytes()
}

var moduleFiles = []file{
	{"manifest.yaml", "name: demo\n"},
	{"scenarios/", ""},
	{"scenarios/a.yaml", "scenario: a\n"},
	{"assets/body.bin", "0123456789"},
}

func TestKeyIDIsDerivedFromThePublicKey(t *testing.T) {
	a, b := signer(t, 1), signer(t, 2)
	if a.KeyID() != signing.KeyID(a.PublicKey()) {
		t.Fatal("signer id differs from KeyID")
	}
	if a.KeyID() == b.KeyID() {
		t.Fatal("two keys share an id")
	}
	if !strings.HasPrefix(a.KeyID(), signing.KeyIDPrefix) || len(a.KeyID()) != len(signing.KeyIDPrefix)+32 {
		t.Fatalf("unexpected key id %q", a.KeyID())
	}
}

// TestGoldenVectors pins the signature format, so that another implementation,
// such as the agent, can check its verifier against fixed bytes.
func TestGoldenVectors(t *testing.T) {
	s := signer(t, 7)
	const digest = "sha256:0000000000000000000000000000000000000000000000000000000000000001"
	sig, err := s.SignDigest(digest)
	if err != nil {
		t.Fatal(err)
	}
	if !ed25519.Verify(s.PublicKey(), []byte(digest), sig.Bytes) {
		t.Fatal("signature is not an Ed25519 signature over the digest text")
	}
	if got := hexOf(sig.Bytes); got != goldenDigestSignature {
		t.Fatalf("digest signature changed:\n got %s\nwant %s", got, goldenDigestSignature)
	}
	if s.KeyID() != goldenKeyID {
		t.Fatalf("key id changed: got %s want %s", s.KeyID(), goldenKeyID)
	}
	idx := signing.NewIndex(3, []signing.IndexEntry{
		{Name: "http1", Version: "1.2.0", Digest: digest},
		{Name: "dns", Version: "0.1.0", Digest: digest},
	})
	canon, err := idx.Canonical()
	if err != nil {
		t.Fatal(err)
	}
	if string(canon) != goldenCanonicalIndex {
		t.Fatalf("canonical index changed:\n got %s\nwant %s", canon, goldenCanonicalIndex)
	}
	si, err := s.SignIndex(idx)
	if err != nil {
		t.Fatal(err)
	}
	if got := hexOf(si.Signature); got != goldenIndexSignature {
		t.Fatalf("index signature changed:\n got %s\nwant %s", got, goldenIndexSignature)
	}
}

func TestSignDigestRefusesAMalformedDigest(t *testing.T) {
	s := signer(t, 1)
	for _, d := range []string{"", "sha256:abc", "md5:" + strings.Repeat("0", 64), "sha256:" + strings.Repeat("G", 64), "sha256:" + strings.Repeat("A", 64)} {
		if _, err := s.SignDigest(d); !errors.Is(err, signing.ErrDigest) {
			t.Errorf("SignDigest(%q) error = %v, want ErrDigest", d, err)
		}
	}
}

func TestGeneratedKeyRoundTripsThroughPEM(t *testing.T) {
	s, err := signing.GenerateSigner(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	priv, err := s.MarshalPEM()
	if err != nil {
		t.Fatal(err)
	}
	back, err := signing.ParseSignerPEM(priv)
	if err != nil {
		t.Fatal(err)
	}
	if back.KeyID() != s.KeyID() {
		t.Fatal("private key id changed in the round trip")
	}
	pubPEM, err := signing.MarshalPublicKeyPEM(s.PublicKey())
	if err != nil {
		t.Fatal(err)
	}
	pub, err := signing.ParsePublicKeyPEM(pubPEM)
	if err != nil {
		t.Fatal(err)
	}
	if !pub.Equal(s.PublicKey()) {
		t.Fatal("public key changed in the round trip")
	}
	for name, fn := range map[string]func() error{
		"private garbage": func() error { _, err := signing.ParseSignerPEM([]byte("nope")); return err },
		"private as public": func() error {
			_, err := signing.ParsePublicKeyPEM(priv)
			return err
		},
		"public as private": func() error {
			_, err := signing.ParseSignerPEM(pubPEM)
			return err
		},
		"short private": func() error { _, err := signing.NewSigner(ed25519.PrivateKey{1, 2}); return err },
		"short public":  func() error { _, err := signing.MarshalPublicKeyPEM(ed25519.PublicKey{1}); return err },
	} {
		if err := fn(); !errors.Is(err, signing.ErrKey) {
			t.Errorf("%s: error = %v, want ErrKey", name, err)
		}
	}
}

func TestArchiveSignatureVerifies(t *testing.T) {
	s := signer(t, 1)
	v := verifier(trust(t, s), epoch)
	for _, gz := range []bool{false, true} {
		data := archive(t, gz, moduleFiles...)
		digest, sig, err := s.SignArchive(bytes.NewReader(data), signing.Limits{})
		if err != nil {
			t.Fatalf("gz=%v: %v", gz, err)
		}
		got, err := v.VerifyArchive(bytes.NewReader(data), sig, signing.Limits{})
		if err != nil || got != digest {
			t.Fatalf("gz=%v: VerifyArchive = %q, %v; want %q", gz, got, err, digest)
		}
		if err := v.VerifyDigest(digest, sig); err != nil {
			t.Fatalf("gz=%v: VerifyDigest: %v", gz, err)
		}
	}
}

func TestArchiveDigestIgnoresLayoutNotContent(t *testing.T) {
	a, err := signing.ArchiveDigest(bytes.NewReader(archive(t, false, moduleFiles...)), signing.Limits{})
	if err != nil {
		t.Fatal(err)
	}
	// Same files in another order, gzip compressed, with ./ prefixes and no
	// directory entries.
	b, err := signing.ArchiveDigest(bytes.NewReader(archive(t, true,
		file{"./assets/body.bin", "0123456789"},
		file{"./scenarios/a.yaml", "scenario: a\n"},
		file{"./manifest.yaml", "name: demo\n"})), signing.Limits{})
	if err != nil {
		t.Fatal(err)
	}
	if a != b {
		t.Fatalf("digest depends on archive layout: %s vs %s", a, b)
	}
}

func TestModifiedArchiveFails(t *testing.T) {
	s := signer(t, 1)
	v := verifier(trust(t, s), epoch)
	orig := archive(t, true, moduleFiles...)
	_, sig, err := s.SignArchive(bytes.NewReader(orig), signing.Limits{})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := v.VerifyArchive(bytes.NewReader(orig), sig, signing.Limits{}); err != nil {
		t.Fatalf("the original must verify: %v", err)
	}
	mods := map[string][]file{
		"changed byte":  {{"manifest.yaml", "name: demo\n"}, {"scenarios/a.yaml", "scenario: b\n"}, {"assets/body.bin", "0123456789"}},
		"added file":    append(append([]file(nil), moduleFiles...), file{"extra.sh", "evil"}),
		"removed file":  moduleFiles[:3],
		"renamed file":  {{"manifest.yaml", "name: demo\n"}, {"scenarios/b.yaml", "scenario: a\n"}, {"assets/body.bin", "0123456789"}},
		"emptied file":  {{"manifest.yaml", ""}, {"scenarios/a.yaml", "scenario: a\n"}, {"assets/body.bin", "0123456789"}},
		"swapped files": {{"manifest.yaml", "name: demo\n"}, {"scenarios/a.yaml", "0123456789"}, {"assets/body.bin", "scenario: a\n"}},
	}
	for name, files := range mods {
		for _, gz := range []bool{false, true} {
			_, err := v.VerifyArchive(bytes.NewReader(archive(t, gz, files...)), sig, signing.Limits{})
			if !errors.Is(err, signing.ErrBadSignature) {
				t.Errorf("%s (gz=%v): error = %v, want ErrBadSignature", name, gz, err)
			}
		}
	}
}

func TestTruncatedArchiveFails(t *testing.T) {
	s := signer(t, 1)
	v := verifier(trust(t, s), epoch)
	raw := archive(t, false, moduleFiles...)
	_, sig, err := s.SignArchive(bytes.NewReader(raw), signing.Limits{})
	if err != nil {
		t.Fatal(err)
	}
	for _, cut := range []int{len(raw) - 1, len(raw) / 2, 700, 100} {
		if _, err := v.VerifyArchive(bytes.NewReader(raw[:cut]), sig, signing.Limits{}); err == nil {
			t.Errorf("an archive cut at %d of %d bytes verified", cut, len(raw))
		}
	}
	gz := archive(t, true, moduleFiles...)
	if _, err := v.VerifyArchive(bytes.NewReader(gz[:len(gz)/2]), sig, signing.Limits{}); err == nil {
		t.Error("a truncated gzip archive verified")
	}
}

func TestArchiveRulesMatchInstall(t *testing.T) {
	big := []file{{"manifest.yaml", strings.Repeat("x", 100)}}
	cases := []struct {
		name  string
		data  []byte
		lim   signing.Limits
		wantE error
	}{
		{"absolute path", archive(t, false, file{"/etc/passwd", "x"}), signing.Limits{}, signing.ErrArchive},
		{"dot dot", archive(t, false, file{"a/../b", "x"}), signing.Limits{}, signing.ErrArchive},
		{"space in name", archive(t, false, file{"a b", "x"}), signing.Limits{}, signing.ErrArchive},
		{"duplicate", archive(t, false, file{"a", "x"}, file{"./a", "y"}), signing.Limits{}, signing.ErrArchive},
		{"not a tar", []byte(strings.Repeat("junk", 300)), signing.Limits{}, signing.ErrArchive},
		{"bad gzip", []byte{0x1f, 0x8b, 0, 0}, signing.Limits{}, signing.ErrArchive},
		{"too many files", archive(t, false, file{"a", "1"}, file{"b", "2"}), signing.Limits{MaxBytes: 100, MaxFiles: 1}, signing.ErrArchiveTooLarge},
		{"too many bytes", archive(t, false, big...), signing.Limits{MaxBytes: 99, MaxFiles: 5}, signing.ErrArchiveTooLarge},
	}
	for _, c := range cases {
		if _, err := signing.ArchiveDigest(bytes.NewReader(c.data), c.lim); !errors.Is(err, c.wantE) {
			t.Errorf("%s: error = %v, want %v", c.name, err, c.wantE)
		}
	}
	var buf bytes.Buffer
	tw := tar.NewWriter(&buf)
	if err := tw.WriteHeader(&tar.Header{Name: "link", Typeflag: tar.TypeSymlink, Linkname: "/etc/passwd"}); err != nil {
		t.Fatal(err)
	}
	if err := tw.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := signing.ArchiveDigest(&buf, signing.Limits{}); !errors.Is(err, signing.ErrArchive) {
		t.Errorf("symlink: error = %v, want ErrArchive", err)
	}
	s := signer(t, 1)
	if _, _, err := s.SignArchive(strings.NewReader("junk"), signing.Limits{}); !errors.Is(err, signing.ErrArchive) {
		t.Errorf("SignArchive of junk: error = %v, want ErrArchive", err)
	}
	v := verifier(trust(t, s), epoch)
	if _, err := v.VerifyArchive(strings.NewReader("junk"), signing.Signature{}, signing.Limits{}); !errors.Is(err, signing.ErrArchive) {
		t.Errorf("VerifyArchive of junk: error = %v, want ErrArchive", err)
	}
}

func TestSignatureFailures(t *testing.T) {
	a, b := signer(t, 1), signer(t, 2)
	stranger := signer(t, 3)
	ts := trust(t, a, b)
	revoked := signer(t, 4)
	if _, err := ts.Add(revoked.PublicKey()); err != nil {
		t.Fatal(err)
	}
	if err := ts.Revoke(revoked.KeyID()); err != nil {
		t.Fatal(err)
	}
	v := verifier(ts, epoch)
	digest := "sha256:" + strings.Repeat("ab", 32)
	good, err := a.SignDigest(digest)
	if err != nil {
		t.Fatal(err)
	}
	if err := v.VerifyDigest(digest, good); err != nil {
		t.Fatalf("the original must verify: %v", err)
	}
	other, _ := stranger.SignDigest(digest)
	byRevoked, _ := revoked.SignDigest(digest)
	byB, _ := b.SignDigest(digest)
	flipped := append([]byte(nil), good.Bytes...)
	flipped[10] ^= 1
	cases := []struct {
		name   string
		digest string
		sig    signing.Signature
		want   error
	}{
		{"unknown key", digest, other, signing.ErrUnknownKey},
		{"revoked key", digest, byRevoked, signing.ErrKeyRevoked},
		{"key id of another trusted key", digest, signing.Signature{KeyID: b.KeyID(), Bytes: good.Bytes}, signing.ErrBadSignature},
		{"key id of the other key, swapped", digest, signing.Signature{KeyID: a.KeyID(), Bytes: byB.Bytes}, signing.ErrBadSignature},
		{"key id that names no key", digest, signing.Signature{KeyID: "ed25519:00000000000000000000000000000000", Bytes: good.Bytes}, signing.ErrUnknownKey},
		{"empty key id", digest, signing.Signature{Bytes: good.Bytes}, signing.ErrUnknownKey},
		{"truncated signature", digest, signing.Signature{KeyID: a.KeyID(), Bytes: good.Bytes[:63]}, signing.ErrSignatureLength},
		{"one byte signature", digest, signing.Signature{KeyID: a.KeyID(), Bytes: good.Bytes[:1]}, signing.ErrSignatureLength},
		{"long signature", digest, signing.Signature{KeyID: a.KeyID(), Bytes: append(append([]byte(nil), good.Bytes...), 0)}, signing.ErrSignatureLength},
		{"flipped signature bit", digest, signing.Signature{KeyID: a.KeyID(), Bytes: flipped}, signing.ErrBadSignature},
		{"other digest", "sha256:" + strings.Repeat("cd", 32), good, signing.ErrBadSignature},
		{"unsigned", digest, signing.Signature{}, signing.ErrNoSignature},
		{"key id without signature", digest, signing.Signature{KeyID: a.KeyID()}, signing.ErrSignatureLength},
		{"malformed digest", "sha256:zz", good, signing.ErrDigest},
	}
	for _, c := range cases {
		if err := v.VerifyDigest(c.digest, c.sig); !errors.Is(err, c.want) {
			t.Errorf("%s: error = %v, want %v", c.name, err, c.want)
		}
	}
}

func TestTrustStoreLifecycle(t *testing.T) {
	old, cur, next := signer(t, 1), signer(t, 2), signer(t, 3)
	ts := trust(t, old, cur)
	digest := "sha256:" + strings.Repeat("ab", 32)
	sigOld, _ := old.SignDigest(digest)
	sigCur, _ := cur.SignDigest(digest)
	sigNext, _ := next.SignDigest(digest)
	cutoff := epoch.Add(24 * time.Hour)

	// Two active keys verify at once.
	if err := verifier(ts, epoch).VerifyDigest(digest, sigOld); err != nil {
		t.Fatalf("active old key: %v", err)
	}
	if err := verifier(ts, epoch).VerifyDigest(digest, sigCur); err != nil {
		t.Fatalf("active current key: %v", err)
	}
	if err := verifier(ts, epoch).VerifyDigest(digest, sigNext); !errors.Is(err, signing.ErrUnknownKey) {
		t.Fatalf("key not yet added: %v", err)
	}

	// Rotation: add the next key, retire the old one with a cutoff.
	if _, err := ts.Add(next.PublicKey()); err != nil {
		t.Fatal(err)
	}
	if err := ts.Retire(old.KeyID(), cutoff); err != nil {
		t.Fatal(err)
	}
	for _, c := range []struct {
		name string
		at   time.Time
		want error
	}{
		{"well before the cutoff", epoch, nil},
		{"one second before the cutoff", cutoff.Add(-time.Second), nil},
		{"at the cutoff", cutoff, signing.ErrKeyExpired},
		{"after the cutoff", cutoff.Add(time.Hour), signing.ErrKeyExpired},
	} {
		if err := verifier(ts, c.at).VerifyDigest(digest, sigOld); !errors.Is(err, c.want) && (c.want != nil || err != nil) {
			t.Errorf("retired key %s: error = %v, want %v", c.name, err, c.want)
		}
	}
	if err := verifier(ts, cutoff.Add(time.Hour)).VerifyDigest(digest, sigNext); err != nil {
		t.Errorf("new key after the cutoff: %v", err)
	}

	// Revocation wins over everything, before and after the cutoff.
	if err := ts.Revoke(old.KeyID()); err != nil {
		t.Fatal(err)
	}
	for _, at := range []time.Time{epoch.Add(-48 * time.Hour), epoch, cutoff.Add(time.Hour)} {
		if err := verifier(ts, at).VerifyDigest(digest, sigOld); !errors.Is(err, signing.ErrKeyRevoked) {
			t.Errorf("revoked key at %s: error = %v, want ErrKeyRevoked", at, err)
		}
	}
	// A revoked key cannot be retired or added again.
	if err := ts.Retire(old.KeyID(), cutoff); !errors.Is(err, signing.ErrKeyRevoked) {
		t.Errorf("retire revoked: %v", err)
	}
	if _, err := ts.Add(old.PublicKey()); !errors.Is(err, signing.ErrTrustStore) {
		t.Errorf("re-add revoked: %v", err)
	}
	// Other refusals.
	if err := ts.Retire(cur.KeyID(), time.Time{}); !errors.Is(err, signing.ErrTrustStore) {
		t.Errorf("retire without cutoff: %v", err)
	}
	if err := ts.Retire("ed25519:nope", cutoff); !errors.Is(err, signing.ErrUnknownKey) {
		t.Errorf("retire unknown: %v", err)
	}
	if err := ts.Revoke("ed25519:nope"); !errors.Is(err, signing.ErrUnknownKey) {
		t.Errorf("revoke unknown: %v", err)
	}
	if _, err := ts.Add(ed25519.PublicKey{1}); !errors.Is(err, signing.ErrKey) {
		t.Errorf("add short key: %v", err)
	}
	// A retired key can get a new cutoff.
	if err := ts.Retire(cur.KeyID(), cutoff); err != nil {
		t.Fatal(err)
	}
	later := cutoff.Add(48 * time.Hour)
	if err := ts.Retire(cur.KeyID(), later); err != nil {
		t.Fatal(err)
	}
	if err := verifier(ts, cutoff.Add(time.Hour)).VerifyDigest(digest, sigCur); err != nil {
		t.Errorf("extended cutoff: %v", err)
	}
	if k, ok := ts.Key(cur.KeyID()); !ok || k.Status != signing.StatusRetired {
		t.Errorf("Key = %+v, %v", k, ok)
	}
	if got := len(ts.Keys()); got != 3 {
		t.Errorf("Keys has %d entries, want 3", got)
	}
}

func TestTrustStoreFileRoundTrip(t *testing.T) {
	a, b, c := signer(t, 1), signer(t, 2), signer(t, 3)
	ts := trust(t, a, b, c)
	cutoff := time.Date(2027, 1, 2, 3, 4, 5, 0, time.UTC)
	if err := ts.Retire(b.KeyID(), cutoff); err != nil {
		t.Fatal(err)
	}
	if err := ts.Revoke(c.KeyID()); err != nil {
		t.Fatal(err)
	}
	data, err := ts.Encode()
	if err != nil {
		t.Fatal(err)
	}
	back, err := signing.ParseTrustStore(data)
	if err != nil {
		t.Fatalf("%v\n%s", err, data)
	}
	again, err := back.Encode()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(data, again) {
		t.Fatalf("encode is not stable:\n%s\n%s", data, again)
	}
	digest := "sha256:" + strings.Repeat("ab", 32)
	for s, want := range map[*signing.Signer]error{a: nil, b: nil, c: signing.ErrKeyRevoked} {
		sig, _ := s.SignDigest(digest)
		err := verifier(back, cutoff.Add(-time.Hour)).VerifyDigest(digest, sig)
		if !errors.Is(err, want) && (want != nil || err != nil) {
			t.Errorf("%s after reload: %v, want %v", s.KeyID(), err, want)
		}
	}
	sigB, _ := b.SignDigest(digest)
	if err := verifier(back, cutoff).VerifyDigest(digest, sigB); !errors.Is(err, signing.ErrKeyExpired) {
		t.Errorf("retired key at its cutoff after reload: %v", err)
	}
	empty, err := signing.NewTrustStore().Encode()
	if err != nil || !strings.Contains(string(empty), `"keys": []`) {
		t.Errorf("empty store encodes as %q, %v", empty, err)
	}
}

func TestParseTrustStoreRefusesBadFiles(t *testing.T) {
	a := signer(t, 1)
	pubB64 := b64(a.PublicKey())
	id := a.KeyID()
	key := func(fields string) string {
		return `{"version":1,"keys":[{` + fields + `}]}`
	}
	cases := map[string]string{
		"not json":          "nope",
		"trailing data":     `{"version":1,"keys":[]} {}`,
		"unknown field":     `{"version":1,"keys":[],"extra":1}`,
		"wrong version":     `{"version":2,"keys":[]}`,
		"id not derived":    key(`"key_id":"ed25519:00000000000000000000000000000000","public_key":"` + pubB64 + `","status":"active"`),
		"short public key":  key(`"key_id":"` + id + `","public_key":"AAAA","status":"active"`),
		"unknown status":    key(`"key_id":"` + id + `","public_key":"` + pubB64 + `","status":"trusted"`),
		"retired no cutoff": key(`"key_id":"` + id + `","public_key":"` + pubB64 + `","status":"retired"`),
		"retired bad time":  key(`"key_id":"` + id + `","public_key":"` + pubB64 + `","status":"retired","retired_after":"soon"`),
		"active cutoff":     key(`"key_id":"` + id + `","public_key":"` + pubB64 + `","status":"active","retired_after":"2027-01-01T00:00:00Z"`),
		"revoked cutoff":    key(`"key_id":"` + id + `","public_key":"` + pubB64 + `","status":"revoked","retired_after":"2027-01-01T00:00:00Z"`),
		"duplicate": `{"version":1,"keys":[{"key_id":"` + id + `","public_key":"` + pubB64 + `","status":"active"},` +
			`{"key_id":"` + id + `","public_key":"` + pubB64 + `","status":"revoked"}]}`,
	}
	for name, doc := range cases {
		if _, err := signing.ParseTrustStore([]byte(doc)); !errors.Is(err, signing.ErrTrustStore) {
			t.Errorf("%s: error = %v, want ErrTrustStore", name, err)
		}
	}
}

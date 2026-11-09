// SPDX-License-Identifier: GPL-3.0-only

package signing_test

import (
	"bytes"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/registry/signing"
)

func hexOf(b []byte) string { return hex.EncodeToString(b) }

func b64(b []byte) string { return base64.StdEncoding.EncodeToString(b) }

func dg(c string) string { return "sha256:" + strings.Repeat(c, 64) }

func sampleIndex() signing.Index {
	return signing.NewIndex(5, []signing.IndexEntry{
		{Name: "http1", Version: "1.0.0", Digest: dg("a")},
		{Name: "dns", Version: "0.2.0", Digest: dg("b")},
		{Name: "dns", Version: "0.10.0", Digest: dg("c")},
		{Name: "http1", Version: "1.0.0-rc.1", Digest: dg("d")},
	})
}

func TestCanonicalIndexIsSortedAndStable(t *testing.T) {
	got, err := sampleIndex().Canonical()
	if err != nil {
		t.Fatal(err)
	}
	// Versions sort as bytes, not as semver: "0.10.0" before "0.2.0", and
	// "1.0.0" before "1.0.0-rc.1" because it is a prefix of it.
	want := `{"format":"fps-registry-index/1","modules":[` +
		`{"digest":"` + dg("c") + `","name":"dns","version":"0.10.0"},` +
		`{"digest":"` + dg("b") + `","name":"dns","version":"0.2.0"},` +
		`{"digest":"` + dg("a") + `","name":"http1","version":"1.0.0"},` +
		`{"digest":"` + dg("d") + `","name":"http1","version":"1.0.0-rc.1"}],"serial":5}`
	if string(got) != want {
		t.Fatalf("canonical form:\n got %s\nwant %s", got, want)
	}
	// Any input order gives the same bytes.
	rev := sampleIndex()
	for i, j := 0, len(rev.Modules)-1; i < j; i, j = i+1, j-1 {
		rev.Modules[i], rev.Modules[j] = rev.Modules[j], rev.Modules[i]
	}
	again, err := rev.Canonical()
	if err != nil || !bytes.Equal(got, again) {
		t.Fatalf("canonical form depends on input order: %v\n%s\n%s", err, got, again)
	}
	empty, err := signing.NewIndex(0, nil).Canonical()
	if err != nil || string(empty) != `{"format":"fps-registry-index/1","modules":[],"serial":0}` {
		t.Fatalf("empty index = %s, %v", empty, err)
	}
}

func TestCanonicalRefusesBadIndexes(t *testing.T) {
	ok := signing.IndexEntry{Name: "dns", Version: "1.0.0", Digest: dg("a")}
	bad := func(mod func(*signing.IndexEntry)) signing.Index {
		e := ok
		mod(&e)
		return signing.NewIndex(1, []signing.IndexEntry{e})
	}
	cases := map[string]signing.Index{
		"unknown format":    {Format: "fps-registry-index/2", Modules: []signing.IndexEntry{ok}},
		"no format":         {Modules: []signing.IndexEntry{ok}},
		"upper case name":   bad(func(e *signing.IndexEntry) { e.Name = "Dns" }),
		"quote in name":     bad(func(e *signing.IndexEntry) { e.Name = `d"ns` }),
		"empty name":        bad(func(e *signing.IndexEntry) { e.Name = "" }),
		"unicode name":      bad(func(e *signing.IndexEntry) { e.Name = "dné" }),
		"two part version":  bad(func(e *signing.IndexEntry) { e.Version = "1.0" }),
		"v prefix":          bad(func(e *signing.IndexEntry) { e.Version = "v1.0.0" }),
		"quote in version":  bad(func(e *signing.IndexEntry) { e.Version = `1.0.0-"x` }),
		"short digest":      bad(func(e *signing.IndexEntry) { e.Digest = "sha256:abc" }),
		"upper case digest": bad(func(e *signing.IndexEntry) { e.Digest = dg("A") }),
		"no digest":         bad(func(e *signing.IndexEntry) { e.Digest = "" }),
		"duplicate":         signing.NewIndex(1, []signing.IndexEntry{ok, ok}),
	}
	for name, idx := range cases {
		if _, err := idx.Canonical(); !errors.Is(err, signing.ErrIndex) {
			t.Errorf("%s: error = %v, want ErrIndex", name, err)
		}
		if _, err := signer(t, 1).SignIndex(idx); !errors.Is(err, signing.ErrIndex) {
			t.Errorf("%s: SignIndex error = %v, want ErrIndex", name, err)
		}
	}
}

func TestIndexFind(t *testing.T) {
	idx := sampleIndex()
	if e, ok := idx.Find("dns", "0.2.0"); !ok || e.Digest != dg("b") {
		t.Fatalf("Find = %+v, %v", e, ok)
	}
	if _, ok := idx.Find("dns", "9.9.9"); ok {
		t.Fatal("Find returned a module that is not listed")
	}
}

func TestSignedIndexVerifies(t *testing.T) {
	s := signer(t, 1)
	v := verifier(trust(t, s), epoch)
	si, err := s.SignIndex(sampleIndex())
	if err != nil {
		t.Fatal(err)
	}
	got, err := v.VerifyIndex(si)
	if err != nil || got.Serial != 5 || len(got.Modules) != 4 {
		t.Fatalf("VerifyIndex = %+v, %v", got, err)
	}
	// The encoded file verifies after a round trip, and its layout is free.
	data, err := si.Encode()
	if err != nil {
		t.Fatal(err)
	}
	parsed, err := signing.ParseSignedIndex(data)
	if err != nil {
		t.Fatalf("%v\n%s", err, data)
	}
	if _, err := v.VerifyIndex(parsed); err != nil {
		t.Fatalf("round trip: %v", err)
	}
	reflowed := strings.NewReplacer("\n", "", "  ", "").Replace(string(data))
	parsed, err = signing.ParseSignedIndex([]byte(reflowed))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := v.VerifyIndex(parsed); err != nil {
		t.Fatalf("reformatted file: %v", err)
	}
}

func TestModifiedIndexFails(t *testing.T) {
	s := signer(t, 1)
	v := verifier(trust(t, s), epoch)
	orig, err := s.SignIndex(sampleIndex())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := v.VerifyIndex(orig); err != nil {
		t.Fatalf("the original must verify: %v", err)
	}
	mods := map[string]func(*signing.SignedIndex){
		"changed digest":  func(si *signing.SignedIndex) { si.Index.Modules[0].Digest = dg("e") },
		"changed version": func(si *signing.SignedIndex) { si.Index.Modules[1].Version = "0.2.1" },
		"changed name":    func(si *signing.SignedIndex) { si.Index.Modules[1].Name = "dns2" },
		"added module": func(si *signing.SignedIndex) {
			si.Index.Modules = append(si.Index.Modules, signing.IndexEntry{Name: "evil", Version: "1.0.0", Digest: dg("f")})
		},
		"removed module": func(si *signing.SignedIndex) { si.Index.Modules = si.Index.Modules[1:] },
		"changed serial": func(si *signing.SignedIndex) { si.Index.Serial-- },
		"flipped signature bit": func(si *signing.SignedIndex) {
			si.Signature = append([]byte(nil), si.Signature...)
			si.Signature[0] ^= 0x80
		},
	}
	for name, mod := range mods {
		si := orig
		si.Index.Modules = append([]signing.IndexEntry(nil), orig.Index.Modules...)
		mod(&si)
		if _, err := v.VerifyIndex(si); !errors.Is(err, signing.ErrBadSignature) {
			t.Errorf("%s: error = %v, want ErrBadSignature", name, err)
		}
	}
	// The same changes made on the encoded file.
	data, err := orig.Encode()
	if err != nil {
		t.Fatal(err)
	}
	text := string(data)
	for name, edited := range map[string]string{
		"file digest":  strings.Replace(text, dg("a"), dg("9"), 1),
		"file version": strings.Replace(text, `"0.2.0"`, `"0.2.1"`, 1),
		"file serial":  strings.Replace(text, `"serial": 5`, `"serial": 6`, 1),
	} {
		if edited == text {
			t.Fatalf("%s: the edit did not apply", name)
		}
		si, err := signing.ParseSignedIndex([]byte(edited))
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if _, err := v.VerifyIndex(si); !errors.Is(err, signing.ErrBadSignature) {
			t.Errorf("%s: error = %v, want ErrBadSignature", name, err)
		}
	}
}

func TestIndexKeyAndSignatureFailures(t *testing.T) {
	a, b, stranger := signer(t, 1), signer(t, 2), signer(t, 3)
	ts := trust(t, a, b)
	revoked := signer(t, 4)
	if _, err := ts.Add(revoked.PublicKey()); err != nil {
		t.Fatal(err)
	}
	if err := ts.Revoke(revoked.KeyID()); err != nil {
		t.Fatal(err)
	}
	v := verifier(ts, epoch)
	good, _ := a.SignIndex(sampleIndex())
	byStranger, _ := stranger.SignIndex(sampleIndex())
	byRevoked, _ := revoked.SignIndex(sampleIndex())
	wrongID := good
	wrongID.KeyID = b.KeyID()
	truncated := good
	truncated.Signature = good.Signature[:40]
	unsigned := good
	unsigned.KeyID, unsigned.Signature = "", nil
	archiveSig, _ := a.SignDigest(dg("a"))
	asArchiveSig := good
	asArchiveSig.Signature = archiveSig.Bytes
	for name, c := range map[string]struct {
		si   signing.SignedIndex
		want error
	}{
		"unknown key":                 {byStranger, signing.ErrUnknownKey},
		"revoked key":                 {byRevoked, signing.ErrKeyRevoked},
		"wrong key id":                {wrongID, signing.ErrBadSignature},
		"truncated signature":         {truncated, signing.ErrSignatureLength},
		"unsigned":                    {unsigned, signing.ErrNoSignature},
		"an archive signature":        {asArchiveSig, signing.ErrBadSignature},
		"unknown format":              {signing.SignedIndex{Index: signing.Index{Format: "x"}, KeyID: a.KeyID(), Signature: good.Signature}, signing.ErrIndex},
		"index with a duplicate name": {signing.SignedIndex{Index: signing.NewIndex(1, []signing.IndexEntry{{Name: "a", Version: "1.0.0", Digest: dg("a")}, {Name: "a", Version: "1.0.0", Digest: dg("b")}}), KeyID: a.KeyID(), Signature: good.Signature}, signing.ErrIndex},
	} {
		if _, err := v.VerifyIndex(c.si); !errors.Is(err, c.want) {
			t.Errorf("%s: error = %v, want %v", name, err, c.want)
		}
	}
	// An archive signature must not verify an index even when the digest
	// text happens to be what an attacker wants signed.
	if err := v.VerifyDigest(dg("a"), archiveSig); err != nil {
		t.Fatalf("control: %v", err)
	}
}

func TestIndexUnderRetiredKey(t *testing.T) {
	old := signer(t, 1)
	ts := trust(t, old)
	cutoff := epoch.Add(time.Hour)
	if err := ts.Retire(old.KeyID(), cutoff); err != nil {
		t.Fatal(err)
	}
	si, _ := old.SignIndex(sampleIndex())
	if _, err := verifier(ts, epoch).VerifyIndex(si); err != nil {
		t.Errorf("before the cutoff: %v", err)
	}
	if _, err := verifier(ts, cutoff).VerifyIndex(si); !errors.Is(err, signing.ErrKeyExpired) {
		t.Errorf("at the cutoff: %v", err)
	}
}

func TestParseSignedIndexRefusesExtras(t *testing.T) {
	good, _ := signer(t, 1).SignIndex(sampleIndex())
	data, err := good.Encode()
	if err != nil {
		t.Fatal(err)
	}
	for name, doc := range map[string]string{
		"not json":      "x",
		"trailing data": string(data) + "{}",
		"unknown field": strings.Replace(string(data), `"key_id"`, `"note": "unsigned", "key_id"`, 1),
		"negative":      strings.Replace(string(data), `"serial": 5`, `"serial": -5`, 1),
	} {
		if _, err := signing.ParseSignedIndex([]byte(doc)); !errors.Is(err, signing.ErrIndex) {
			t.Errorf("%s: error = %v, want ErrIndex", name, err)
		}
	}
}

func TestNilClockUsesTheWallClock(t *testing.T) {
	s := signer(t, 1)
	ts := trust(t, s)
	sig, _ := s.SignDigest(dg("a"))
	if err := signing.NewVerifier(ts, nil).VerifyDigest(dg("a"), sig); err != nil {
		t.Fatal(err)
	}
}

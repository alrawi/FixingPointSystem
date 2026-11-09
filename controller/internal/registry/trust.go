// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/registry/signing"
)

// ErrUntrusted reports an install that the trust policy refused: the module
// has no signature and the registry requires one, or its signature does not
// verify. The error wraps the cause from the signing package, so callers can
// tell signing.ErrNoSignature, signing.ErrBadSignature, signing.ErrUnknownKey,
// signing.ErrKeyRevoked, and signing.ErrKeyExpired apart with errors.Is.
var ErrUntrusted = errors.New("module signature not accepted")

// TrustPolicy says which module signatures the registry accepts at install.
//
//   - Without a trust store (Store nil and StorePath empty) the registry does
//     not require signatures. It installs every module that passes validation,
//     ignores a signature that comes with the module, and logs a warning, so
//     a controller that nobody configured for signing keeps working. Set a
//     trust store to turn enforcement on.
//   - With a trust store, an install needs a signature that verifies against
//     the store. A modified archive, an unknown, revoked, or expired key, and a
//     bad signature are refused, and so is a module without a signature, unless
//     AllowUnsigned is set. A bad signature is refused even then.
type TrustPolicy struct {
	// Store is a trust store held in memory. It wins over StorePath.
	Store *signing.TrustStore
	// StorePath is the trust store file that signing.TrustStore.Encode writes.
	// The registry reads it at Open, to fail early, and again at every
	// install, so a key that an operator revokes in the file stops verifying
	// at once without a restart.
	StorePath string
	// AllowUnsigned accepts a module that comes without a signature. It is
	// the development setting. It has an effect only with a trust store.
	AllowUnsigned bool
	// Now is the clock for retired keys. Nil means time.Now.
	Now func() time.Time
}

// Enforced reports whether a trust store is configured, which is when the
// registry requires signatures.
func (p TrustPolicy) Enforced() bool { return p.Store != nil || p.StorePath != "" }

// verifier returns a verifier over the current trust store.
func (p TrustPolicy) verifier() (*signing.Verifier, error) {
	store := p.Store
	if store == nil {
		data, err := os.ReadFile(filepath.Clean(p.StorePath))
		if err != nil {
			return nil, fmt.Errorf("read trust store: %w", err)
		}
		if store, err = signing.ParseTrustStore(data); err != nil {
			return nil, fmt.Errorf("trust store %s: %w", p.StorePath, err)
		}
	}
	return signing.NewVerifier(store, p.Now), nil
}

// Option configures Open.
type Option func(*Registry)

// WithTrust sets the trust policy of the registry.
func WithTrust(p TrustPolicy) Option { return func(r *Registry) { r.trust = p } }

// WithLogger sets the logger for the warnings about unsigned installs. The
// default discards them.
func WithLogger(l *slog.Logger) Option {
	return func(r *Registry) {
		if l != nil {
			r.logger = l
		}
	}
}

// InstallOption configures one Install or InstallDir call.
type InstallOption func(*installOptions)

type installOptions struct{ sig signing.Signature }

// WithSignature supplies the detached signature of the module: the signature
// over the module tree digest and the id of the key that made it.
func WithSignature(sig signing.Signature) InstallOption {
	return func(o *installOptions) { o.sig = sig }
}

// signed is the verification that an install must complete.
type signed struct {
	verifier *signing.Verifier
	sig      signing.Signature
	// digest is the digest that VerifyArchive computed before unpacking. It is
	// empty for InstallDir, which verifies the staged tree instead.
	digest string
}

// gate applies the trust policy to the signature of one install. It returns
// nil when the install needs no verification, and an error that wraps
// ErrUntrusted when the policy refuses the module outright.
func (r *Registry) gate(opts []InstallOption) (*signed, error) {
	var o installOptions
	for _, f := range opts {
		f(&o)
	}
	has := len(o.sig.Bytes) > 0 || o.sig.KeyID != ""
	if !r.trust.Enforced() {
		if has {
			r.logger.Warn("module signature ignored: no trust store is configured", "setting", "trust_store")
		}
		return nil, nil
	}
	if !has {
		if r.trust.AllowUnsigned {
			return nil, nil
		}
		return nil, fmt.Errorf("%w: %w; sign the module or set allow_unsigned_modules for development", ErrUntrusted, signing.ErrNoSignature)
	}
	v, err := r.trust.verifier()
	if err != nil {
		return nil, fmt.Errorf("check module signature: %w", err)
	}
	return &signed{verifier: v, sig: o.sig}, nil
}

// refuse classifies an error of the signing package. An archive that cannot be
// read is an archive problem, and everything else is a refusal of the
// signature.
func refuse(err error) error {
	switch {
	case errors.Is(err, signing.ErrArchiveTooLarge):
		return fmt.Errorf("%w: %s", ErrArchiveTooLarge, err.Error())
	case errors.Is(err, signing.ErrArchive):
		return fmt.Errorf("%w: %s", ErrArchive, err.Error())
	default:
		return fmt.Errorf("%w: %w", ErrUntrusted, err)
	}
}

// spool copies the archive into a file in the staging directory, so the
// registry can verify it and then unpack the very bytes that it verified. The
// copy stops one byte past a bound that no archive within the limits can
// exceed, so an endless stream cannot fill the disk.
func (r *Registry) spool(archive io.Reader) (*os.File, error) {
	f, err := os.CreateTemp(r.stagingDir(), "archive-")
	if err != nil {
		return nil, fmt.Errorf("spool archive: %w", err)
	}
	// Tar headers and padding add up to 1 KiB per file on top of the content,
	// and compression only shrinks that for realistic archives.
	bound := r.limits.MaxBytes + int64(r.limits.MaxFiles)*1024
	n, err := io.Copy(f, io.LimitReader(archive, bound+1))
	if err == nil && n > bound {
		err = fmt.Errorf("%w: more than %d bytes", ErrArchiveTooLarge, bound)
	}
	if err == nil {
		_, err = f.Seek(0, io.SeekStart)
	}
	if err != nil {
		_ = f.Close()
		_ = os.Remove(f.Name())
		return nil, fmt.Errorf("spool archive: %w", err)
	}
	return f, nil
}

func (s *signed) encode(e *Entry) {
	e.Signature = base64.StdEncoding.EncodeToString(s.sig.Bytes)
	e.SignatureKeyID = s.sig.KeyID
}

// StoredSignature returns the signature that the registry stored with the
// module, and false for a module that was installed without one.
func (e Entry) StoredSignature() (signing.Signature, bool) {
	if e.Signature == "" {
		return signing.Signature{}, false
	}
	b, err := base64.StdEncoding.DecodeString(e.Signature)
	if err != nil {
		return signing.Signature{}, false
	}
	return signing.Signature{KeyID: e.SignatureKeyID, Bytes: b}, true
}

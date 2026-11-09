// SPDX-License-Identifier: GPL-3.0-only

package compiler

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"
)

// TestPlanCarriesTheModuleSignature gives one module of the catalog a stored
// signature and expects every plan to repeat it with the key id, in the proto3
// JSON names of fps.agent.v1.ModuleRef, and to leave the other modules alone.
func TestPlanCarriesTheModuleSignature(t *testing.T) {
	sig := bytes.Repeat([]byte{0xab}, 64)
	enc := base64.StdEncoding.EncodeToString(sig)
	const keyID = "ed25519:fe812c12f3ab4ce6ac5db69ac352f906"
	c := goldenCases()[6] // module_settings: uses dns and http1
	cat := exampleCatalog(t)
	for i := range cat {
		if cat[i].Module.Name == "dns" {
			cat[i].Module.Signature = enc
			cat[i].Module.SignatureKeyID = keyID
		}
	}
	plans, err := Compile(Request{RunID: "run-0001", Config: loadConfig(t, c.config), Agents: c.agents, Scenarios: cat})
	if err != nil {
		t.Fatal(err)
	}
	signed := 0
	for _, p := range plans {
		for _, m := range p.Modules {
			if m.Name != "dns" {
				if m.Signature != "" || m.SignatureKeyID != "" {
					t.Errorf("module %s has signature %q key %q, want none", m.Name, m.Signature, m.SignatureKeyID)
				}
				continue
			}
			signed++
			if m.Signature != enc || m.SignatureKeyID != keyID {
				t.Errorf("module dns has signature %q key %q, want %q key %q", m.Signature, m.SignatureKeyID, enc, keyID)
			}
		}
		b, err := p.MarshalCanonical()
		if err != nil {
			t.Fatal(err)
		}
		var doc struct {
			Modules []struct {
				Name           string `json:"name"`
				Signature      []byte `json:"signature"`
				SignatureKeyID string `json:"signatureKeyId"`
			} `json:"modules"`
		}
		if err := json.Unmarshal(b, &doc); err != nil {
			t.Fatal(err)
		}
		for _, m := range doc.Modules {
			if (m.Name == "dns") != (len(m.Signature) > 0) {
				t.Errorf("module %s: JSON signature %x", m.Name, m.Signature)
			}
			if m.Name == "dns" && (!bytes.Equal(m.Signature, sig) || m.SignatureKeyID != keyID) {
				t.Errorf("module dns: JSON signature %x key %q", m.Signature, m.SignatureKeyID)
			}
		}
	}
	if signed == 0 {
		t.Fatal("no plan carries the dns module")
	}
}

// TestUnsignedModulesAddNoSignatureFields keeps the plans of unsigned modules
// as they were, so the golden plans stay valid.
func TestUnsignedModulesAddNoSignatureFields(t *testing.T) {
	c := goldenCases()[6]
	for _, b := range render(t, compileCase(t, c, c.agents)) {
		if strings.Contains(string(b), `"signature"`) || strings.Contains(string(b), `"signatureKeyId"`) {
			t.Fatalf("plan of unsigned modules mentions a signature: %s", b)
		}
	}
}

// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"context"
	"errors"
	"fmt"
	"path/filepath"
	"strings"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
)

func TestValidateEveryExample(t *testing.T) {
	v := NewValidator(newSchema(t), exampleCatalog(t), exampleInventory())
	for _, ex := range examples(t) {
		rep, err := v.Validate(context.Background(), mustCanonical(t, ex.data))
		if err != nil {
			t.Fatalf("%s: %v", ex.name, err)
		}
		if !rep.Valid || len(rep.Findings) != 0 {
			t.Errorf("%s: report %+v, want valid without findings", ex.name, rep)
		}
		// The spare agent has no interface list, so it counts as having every
		// interface and port_locations sees two available agents.
		if rep.Capacity == nil || *rep.Capacity.AgentsRequired != 1 || *rep.Capacity.AgentsAvailable < 1 {
			t.Errorf("%s: capacity %+v, want 1 required and at least 1 available", ex.name, rep.Capacity)
		}
	}
}

func TestValidateNamesUnresolvedFields(t *testing.T) {
	v := NewValidator(newSchema(t), exampleCatalog(t), exampleInventory())
	for _, fx := range readFixtures(t, filepath.Join("testdata", "unresolved")) {
		rep, err := v.Validate(context.Background(), mustCanonical(t, fx.data))
		if err != nil {
			t.Fatalf("%s: %v", fx.name, err)
		}
		if rep.Valid || !hasFinding(rep.Findings, api.FindingSeverityError, fx.want) {
			t.Errorf("%s: report %+v has no error at %s", fx.name, rep, fx.want)
		}
	}
}

func TestValidateReportsSchemaFindings(t *testing.T) {
	v := NewValidator(newSchema(t), nil, nil)
	for _, fx := range readFixtures(t, filepath.Join("testdata", "invalid")) {
		rep, err := v.Validate(context.Background(), mustCanonical(t, fx.data))
		if err != nil {
			t.Fatalf("%s: %v", fx.name, err)
		}
		if rep.Valid || rep.Capacity != nil || !hasFinding(rep.Findings, api.FindingSeverityError, fx.want) {
			t.Errorf("%s: report %+v has no error at %s", fx.name, rep, fx.want)
		}
	}
}

func TestValidateWithoutDependencies(t *testing.T) {
	v := NewValidator(newSchema(t), nil, nil)
	doc := mustCanonical(t, exampleNamed(t, "enterprise_mix_10g").data)
	rep, err := v.Validate(context.Background(), doc)
	if err != nil {
		t.Fatal(err)
	}
	if !rep.Valid || !hasFinding(rep.Findings, api.FindingSeverityInfo, "/profile/mix") ||
		!hasFinding(rep.Findings, api.FindingSeverityInfo, "/network/ports") {
		t.Errorf("report %+v, want valid with two info findings", rep)
	}
	if rep.Capacity == nil || *rep.Capacity.AgentsRequired != 1 || rep.Capacity.AgentsAvailable != nil {
		t.Errorf("capacity %+v, want 1 required and unknown availability", rep.Capacity)
	}
}

func TestValidateDependencyErrors(t *testing.T) {
	doc := mustCanonical(t, exampleNamed(t, "enterprise_mix_10g").data)
	boom := errors.New("boom")
	for _, v := range []*Validator{
		NewValidator(newSchema(t), fakeCatalog{err: boom}, exampleInventory()),
		NewValidator(newSchema(t), exampleCatalog(t), fakeInventory{err: boom}),
	} {
		if _, err := v.Validate(context.Background(), doc); !errors.Is(err, boom) {
			t.Errorf("Validate = %v, want boom", err)
		}
	}
	if _, err := NewValidator(newSchema(t), nil, nil).Validate(context.Background(), []byte(`{`)); err == nil {
		t.Error("Validate of malformed JSON succeeded")
	}
}

func TestValidateAmbiguousBareScenario(t *testing.T) {
	cat := exampleCatalog(t)
	cat.scenarios = append(cat.scenarios, compiler.Scenario{Module: compiler.ModuleRef{Name: "http2"}, Name: "web_get"})
	v := NewValidator(newSchema(t), cat, exampleInventory())
	ex := examples(t)
	var smoke fixture
	for _, e := range ex {
		if e.name == "http_smoke" {
			smoke = e
		}
	}
	data := strings.Replace(string(smoke.data), "scenario: http1/web_get", "scenario: web_get", 1)
	rep, err := v.Validate(context.Background(), mustCanonical(t, []byte(data)))
	if err != nil {
		t.Fatal(err)
	}
	if rep.Valid || !hasFinding(rep.Findings, api.FindingSeverityError, "/profile/mix/0/scenario") {
		t.Errorf("report %+v, want an ambiguity error", rep)
	}
	data = strings.Replace(string(smoke.data), "scenario: http1/web_get", "scenario: lookup", 1)
	rep, err = v.Validate(context.Background(), mustCanonical(t, []byte(data)))
	if err != nil || !rep.Valid {
		t.Errorf("unique bare name: report %+v, %v, want valid", rep, err)
	}
	data = strings.Replace(string(smoke.data), "scenario: http1/web_get", "scenario: nothing", 1)
	rep, err = v.Validate(context.Background(), mustCanonical(t, []byte(data)))
	if err != nil || rep.Valid || !hasFinding(rep.Findings, api.FindingSeverityError, "/profile/mix/0/scenario") {
		t.Errorf("unknown bare name: report %+v, %v, want an error", rep, err)
	}
}

func TestValidateRunsTheCompiler(t *testing.T) {
	cat := exampleCatalog(t)
	for i := range cat.scenarios {
		cat.scenarios[i].BytesPerSession = 0
	}
	v := NewValidator(newSchema(t), cat, exampleInventory())
	for _, ex := range examples(t) {
		if ex.name != "enterprise_mix_10g" {
			continue
		}
		rep, err := v.Validate(context.Background(), mustCanonical(t, ex.data))
		if err != nil {
			t.Fatal(err)
		}
		if rep.Valid || len(rep.Findings) != 1 || !strings.Contains(rep.Findings[0].Message, "bytes per session") {
			t.Errorf("report %+v, want one compiler error about the byte estimate", rep)
		}
		return
	}
	t.Fatal("example enterprise_mix_10g is missing")
}

func TestCompilerPath(t *testing.T) {
	tests := []struct {
		err  error
		want string
	}{
		{fmt.Errorf("compile test \"x\": %w: network.host_pools[1]: count", compiler.ErrInvalidConfig), "/network/host_pools/1"},
		{fmt.Errorf("compile test \"x\": profile.mix[0].scenario: %w web", compiler.ErrUnknownScenario), "/profile/mix/0/scenario"},
		{fmt.Errorf("%w: load.phases[2].duration_s: too long", compiler.ErrInvalidConfig), "/load/phases/2/duration_s"},
		{fmt.Errorf("compile test \"x\": %w: no agents", compiler.ErrInvalidInventory), ""},
	}
	for _, tc := range tests {
		if got := compilerPath(tc.err); got != tc.want {
			t.Errorf("compilerPath(%v) = %q, want %q", tc.err, got, tc.want)
		}
	}
}

// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const (
	scenarioSchemaPath = contractsDir + "/schemas/scenario.schema.json"
	webProfilePath     = examplesDir + "/http1/scenarios-pending/web_profile.yaml"
)

func TestEmbeddedScenarioSchemaMatchesContract(t *testing.T) {
	want, err := os.ReadFile(scenarioSchemaPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(scenarioSchemaJSON, want) {
		t.Fatal("schema/scenario.schema.json differs from the contract, run go generate ./internal/registry")
	}
}

func webProfile(t *testing.T) string {
	t.Helper()
	data, err := os.ReadFile(webProfilePath)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

// withScenario returns the http1 example with the scenario file at path set to
// src. A path under scenarios/ is listed in the manifest as well.
func withScenario(t *testing.T, path, src string) map[string][]byte {
	t.Helper()
	files := exampleTree(t, "http1", nil)
	files[path] = []byte(src)
	if strings.HasPrefix(path, "scenarios/") {
		m := string(files[ManifestFile])
		m = strings.Replace(m, "  - scenarios/web_page.yaml\n", "  - scenarios/web_page.yaml\n  - "+path+"\n", 1)
		files[ManifestFile] = []byte(m)
	}
	return files
}

func TestInstallScenarioWithProfilesAndDist(t *testing.T) {
	ctx := context.Background()

	// The example module carries web_profile.yaml under scenarios-pending/, so
	// installing it as shipped runs the v0.3 checks on the file.
	t.Run("pending example", func(t *testing.T) {
		if _, err := os.Stat(webProfilePath); err != nil {
			t.Fatal(err)
		}
		r := openRegistry(t)
		e, err := r.InstallDir(ctx, filepath.Join(examplesDir, "http1"))
		if err != nil {
			t.Fatalf("InstallDir(http1) = %v", err)
		}
		if strings.Contains(strings.Join(e.Scenarios, ","), "web_profile") {
			t.Errorf("a pending scenario is indexed: %v", e.Scenarios)
		}
	})

	// A manifest that lists the scenario indexes it.
	t.Run("listed example", func(t *testing.T) {
		r := openRegistry(t)
		files := withScenario(t, "scenarios/web_profile.yaml", webProfile(t))
		e, err := r.Install(ctx, bytes.NewReader(tarFiles(t, files, true)))
		if err != nil {
			t.Fatalf("Install = %v", err)
		}
		if got := strings.Join(e.Scenarios, ","); got != "web_get,web_browse,web_page,web_profile" {
			t.Errorf("scenarios %s", got)
		}
	})
}

func TestInstallRejectsBadScenarios(t *testing.T) {
	ctx := context.Background()
	base := webProfile(t)
	edit := func(old, repl string) string {
		if !strings.Contains(base, old) {
			t.Fatalf("example lacks %q", old)
		}
		return strings.Replace(base, old, repl, 1)
	}
	tests := []struct {
		name string
		src  string
		want string // substring of the finding message, empty for any schema finding
	}{
		{"undeclared profile", edit("headers=profiles.chrome", "headers=profiles.firefox"), "undefined profile 'firefox'"},
		{"bare profiles", edit("headers=profiles.chrome", "headers=profiles"), "must have the form profiles.<name>"},
		{"profile with a field", edit("headers=profiles.chrome", "headers=profiles.chrome.ua"), "must have the form profiles.<name>"},
		{"profile entry with a line break", edit("Accept-Language: en-US,en;q=0.9", "Accept-Language: \"a\\r\\nX: b\""), ""},
		{"empty profile", edit("profiles:\n  chrome:", "profiles:\n  empty: {}\n  chrome:"), ""},
		{"unknown distribution", edit("dist.lognormal(3.0, 0.8)", "dist.gamma(3.0, 0.8)"), "unknown distribution 'gamma'"},
		{"missing parameter", edit("dist.lognormal(3.0, 0.8)", "dist.lognormal(3.0)"), "misses required parameter 'sigma'"},
		{"too many arguments", edit("dist.lognormal(3.0, 0.8)", "dist.lognormal(3.0, 0.8, 5, 6)"), "at most 3 positional"},
		{"unknown keyword", edit("dist.lognormal(3.0, 0.8)", "dist.lognormal(3.0, sigma=0.8, shape=2)"), "no parameter 'shape'"},
		{"parameter twice", edit("dist.lognormal(3.0, 0.8)", "dist.lognormal(3.0, 0.8, sigma=0.9)"), "twice"},
		{"non positive sigma", edit("dist.lognormal(3.0, 0.8)", "dist.lognormal(3.0, 0)"), "'sigma' of 'dist.lognormal' must be positive"},
		{"non positive cap", edit("cap=1048576", "cap=0"), "'cap' of 'dist.pareto' must be positive"},
		{"float cap", edit("cap=1048576", "cap=1.5"), "takes int, got float"},
		{"text argument", edit("dist.lognormal(3.0, 0.8)", "dist.lognormal('a', 0.8)"), "takes float, got text"},
		{"negative low", edit("dist.lognormal(3.0, 0.8)", "dist.uniform(-1, 5)"), "must not be negative"},
		{"low above high", edit("dist.lognormal(3.0, 0.8)", "dist.uniform(9, 5)"), "low greater than high"},
		{"dist without a call", edit("dist.lognormal(3.0, 0.8)", "dist.lognormal"), "call it with parentheses"},
		{"syntax error", edit("dist.lognormal(3.0, 0.8)", "dist.lognormal(3.0,)"), "cannot parse expression"},
		{"unbalanced braces", edit("{{ dist.lognormal(3.0, 0.8) }}", "{{ dist.lognormal(3.0, 0.8) }} }}"), "unbalanced"},
	}
	for _, tt := range tests {
		for _, path := range []string{"scenarios/web_profile.yaml", "scenarios-pending/web_profile.yaml"} {
			t.Run(tt.name+" in "+filepath.Dir(path), func(t *testing.T) {
				r := openRegistry(t)
				// The pending example is part of the base tree, so replace it.
				files := withScenario(t, path, tt.src)
				if path != "scenarios-pending/web_profile.yaml" {
					delete(files, "scenarios-pending/web_profile.yaml")
				}
				_, err := r.Install(ctx, bytes.NewReader(tarFiles(t, files, false)))
				if err == nil {
					t.Fatal("Install succeeded")
				}
				if !strings.Contains(err.Error(), tt.want) { // an empty want matches
					t.Fatalf("error %q lacks %q", err, tt.want)
				}
				paths := findingPaths(t, err)
				if len(paths) == 0 || !strings.HasPrefix(paths[0], "file:"+path+"#/") {
					t.Fatalf("finding paths %v, want one under %s", paths, path)
				}
				if got := r.List(ctx, ""); len(got) != 0 {
					t.Errorf("a rejected install left %+v", got)
				}
			})
		}
	}
}

func TestScenarioMustMatchSchema(t *testing.T) {
	ctx := context.Background()
	// A v0.3 scenario has no other top level key than the ones of the schema.
	src := strings.Replace(webProfile(t), "profiles:", "profile:", 1)
	r := openRegistry(t)
	_, err := r.Install(ctx, bytes.NewReader(tarFiles(t, withScenario(t, "scenarios/web_profile.yaml", src), false)))
	if err == nil || !strings.Contains(err.Error(), "profile") {
		t.Fatalf("Install = %v, want a schema finding about the key", err)
	}
}

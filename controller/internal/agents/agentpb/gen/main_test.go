// SPDX-License-Identifier: GPL-3.0-only

package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"testing"
)

// TestGeneratedCodeIsCurrent regenerates package agentpb from the contract
// and compares the result with the committed files.
func TestGeneratedCodeIsCurrent(t *testing.T) {
	if testing.Short() {
		t.Skip("builds both protoc plugins")
	}
	files, err := Generate(context.Background(), "../../../../../contracts/proto")
	if err != nil {
		t.Fatal(err)
	}
	if len(files) != 2 {
		t.Fatalf("Generate returned %d files, want 2", len(files))
	}
	for _, name := range []string{"agent.pb.go", "agent_grpc.pb.go"} {
		want, ok := files[name]
		if !ok {
			t.Fatalf("Generate wrote no %s", name)
		}
		got, err := os.ReadFile(filepath.Clean(filepath.Join("..", name)))
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(got, want) {
			t.Errorf("internal/agents/agentpb/%s is out of date with the contract; run go generate in controller/internal/agents/agentpb", name)
		}
	}
}

// TestRequestCarriesDependencies checks that the request lists every import
// before the file that needs it, as protoc does.
func TestRequestCarriesDependencies(t *testing.T) {
	req, err := Request(context.Background(), "../../../../../contracts/proto")
	if err != nil {
		t.Fatal(err)
	}
	seen := map[string]bool{}
	for _, f := range req.GetProtoFile() {
		for _, dep := range f.GetDependency() {
			if !seen[dep] {
				t.Fatalf("%s comes before its import %s", f.GetName(), dep)
			}
		}
		seen[f.GetName()] = true
	}
	if !seen[ProtoFile] {
		t.Fatalf("request lacks %s", ProtoFile)
	}
	last := req.GetProtoFile()[len(req.GetProtoFile())-1]
	if last.GetSourceCodeInfo() == nil {
		t.Fatal("the agent proto carries no source info, so generated code would lose its comments")
	}
}

func TestRequestRejectsMissingRoot(t *testing.T) {
	if _, err := Request(context.Background(), t.TempDir()); err == nil {
		t.Fatal("Request accepted a root without the proto file")
	}
}

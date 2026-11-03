// SPDX-License-Identifier: GPL-3.0-only

// Command gen writes the generated code of package agentpb from the agent
// control protocol in contracts/proto/fps/agent/v1/agent.proto. Package
// agentpb runs it through go generate:
//
//	cd controller/internal/agents/agentpb && go generate .
//
// It compiles the proto file with protocompile, so no protoc binary is needed,
// and passes the resulting CodeGeneratorRequest to protoc-gen-go and
// protoc-gen-go-grpc, which go.mod pins as tools and which run through
// go tool. The test TestGeneratedCodeIsCurrent fails when agent.pb.go or
// agent_grpc.pb.go differs from what this command would write.
package main

import (
	"bytes"
	"context"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"path"
	"path/filepath"

	"github.com/bufbuild/protocompile"
	"github.com/bufbuild/protocompile/linker"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/reflect/protodesc"
	"google.golang.org/protobuf/reflect/protoreflect"
	"google.golang.org/protobuf/types/descriptorpb"
	"google.golang.org/protobuf/types/pluginpb"
)

// ProtoFile is the agent control protocol, relative to contracts/proto.
const ProtoFile = "fps/agent/v1/agent.proto"

// ImportPath is the Go import path of package agentpb.
const ImportPath = "github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"

// license starts every generated file. The plugins copy the leading comments
// of the proto file, which already start with it.
const license = "// SPDX-License-Identifier: GPL-3.0-only\n\n"

func main() {
	root := flag.String("proto", "../../../../contracts/proto", "path of the contracts/proto directory")
	out := flag.String("out", ".", "directory of package agentpb")
	flag.Parse()
	files, err := Generate(context.Background(), *root)
	if err == nil {
		err = write(*out, files)
	}
	if err != nil {
		_, _ = fmt.Fprintln(os.Stderr, "gen:", err)
		os.Exit(1)
	}
}

func write(dir string, files map[string][]byte) error {
	for name, data := range files {
		if err := os.WriteFile(filepath.Join(dir, name), data, 0o600); err != nil {
			return fmt.Errorf("write %s: %w", name, err)
		}
	}
	return nil
}

// Generate returns the generated files of package agentpb, keyed by file
// name. It runs both plugins with go tool, so the working directory must lie
// inside the controller module.
func Generate(ctx context.Context, protoRoot string) (map[string][]byte, error) {
	req, err := Request(ctx, protoRoot)
	if err != nil {
		return nil, err
	}
	in, err := proto.Marshal(req)
	if err != nil {
		return nil, fmt.Errorf("marshal code generator request: %w", err)
	}
	files := map[string][]byte{}
	for _, run := range []func(context.Context) *exec.Cmd{protocGenGo, protocGenGoGRPC} {
		if err := runPlugin(run(ctx), in, files); err != nil {
			return nil, err
		}
	}
	return files, nil
}

// protocGenGo and protocGenGoGRPC name the plugins with constant arguments,
// so the command line never depends on input.
func protocGenGo(ctx context.Context) *exec.Cmd {
	return exec.CommandContext(ctx, "go", "tool", "protoc-gen-go")
}

func protocGenGoGRPC(ctx context.Context) *exec.Cmd {
	return exec.CommandContext(ctx, "go", "tool", "protoc-gen-go-grpc")
}

func runPlugin(cmd *exec.Cmd, in []byte, files map[string][]byte) error {
	var stdout, stderr bytes.Buffer
	cmd.Stdin = bytes.NewReader(in)
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("run %s: %w: %s", cmd.Args, err, stderr.String())
	}
	var resp pluginpb.CodeGeneratorResponse
	if err := proto.Unmarshal(stdout.Bytes(), &resp); err != nil {
		return fmt.Errorf("decode response of %s: %w", cmd.Args, err)
	}
	if resp.Error != nil {
		return fmt.Errorf("%s: %s", cmd.Args, resp.GetError())
	}
	for _, f := range resp.GetFile() {
		if f.GetInsertionPoint() != "" {
			return fmt.Errorf("%s: unexpected insertion point in %s", cmd.Args, f.GetName())
		}
		content := []byte(f.GetContent())
		if !bytes.HasPrefix(content, []byte(license[:len(license)-1])) {
			content = append([]byte(license), content...)
		}
		files[path.Base(f.GetName())] = content
	}
	return nil
}

// Request compiles the proto file under protoRoot and returns the request
// that protoc would send to a plugin. The Go package comes from an M option,
// because the contract carries no go_package.
func Request(ctx context.Context, protoRoot string) (*pluginpb.CodeGeneratorRequest, error) {
	c := protocompile.Compiler{
		Resolver: protocompile.WithStandardImports(&protocompile.SourceResolver{
			ImportPaths: []string{protoRoot},
		}),
		SourceInfoMode: protocompile.SourceInfoStandard,
	}
	res, err := c.Compile(ctx, ProtoFile)
	if err != nil {
		return nil, fmt.Errorf("compile %s: %w", ProtoFile, err)
	}
	if len(res) != 1 {
		return nil, fmt.Errorf("compile %s: got %d files, want 1", ProtoFile, len(res))
	}
	agent, ok := res[0].(linker.Result)
	if !ok {
		return nil, errors.New("compile result carries no descriptor proto")
	}
	var protos []*descriptorpb.FileDescriptorProto
	seen := map[string]bool{}
	var walk func(fd protoreflect.FileDescriptor)
	walk = func(fd protoreflect.FileDescriptor) {
		if seen[fd.Path()] {
			return
		}
		seen[fd.Path()] = true
		imports := fd.Imports()
		for i := range imports.Len() {
			walk(imports.Get(i).FileDescriptor)
		}
		if fd.Path() == ProtoFile {
			protos = append(protos, agent.FileDescriptorProto())
			return
		}
		protos = append(protos, protodesc.ToFileDescriptorProto(fd))
	}
	walk(agent)
	param := "paths=source_relative,M" + ProtoFile + "=" + ImportPath + ";agentpb"
	return &pluginpb.CodeGeneratorRequest{
		FileToGenerate: []string{ProtoFile},
		Parameter:      proto.String(param),
		ProtoFile:      protos,
	}, nil
}

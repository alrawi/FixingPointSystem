# SPDX-License-Identifier: GPL-3.0-only
"""Generate fixingpoint/_generated.py from the controller OpenAPI document.

The generator is small on purpose. It reads contracts/openapi/fps-api-v1.yaml and
writes plain TypedDicts for the component schemas and one method per JSON, text,
or binary operation. Server Sent Events operations are left to a later package.
Schemas that live in other files (the test configuration and the module
manifest) become `dict[str, Any]`, because the SDK does not validate them.

    python3 sdk/python/scripts/generate_client.py            # rewrite the client
    python3 sdk/python/scripts/generate_client.py --check    # fail when it drifted

The generator needs PyYAML. The generated client needs only the standard library.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import keyword
import re
import sys
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
SDK_ROOT = HERE.parent
DEFAULT_SPEC = SDK_ROOT.parent.parent / "contracts" / "openapi" / "fps-api-v1.yaml"
DEFAULT_OUTPUT = SDK_ROOT / "fixingpoint" / "_generated.py"

METHODS = ("get", "post", "put", "patch", "delete")


def snake(name: str) -> str:
    out = re.sub(r"(?<=[a-z0-9])([A-Z])", r"_\1", name).lower()
    return out + "_" if keyword.iskeyword(out) else out


def q(value: str) -> str:
    return json.dumps(value)


def pascal(name: str) -> str:
    return "".join(p[:1].upper() + p[1:] for p in re.split(r"[^A-Za-z0-9]+", name) if p)


class Generator:
    def __init__(self, spec: dict[str, Any]) -> None:
        self.spec = spec
        self.schemas: dict[str, dict[str, Any]] = spec.get("components", {}).get("schemas", {})
        self.typed_dicts: dict[str, list[str]] = {}
        self.aliases: dict[str, str] = {}

    # Types ---------------------------------------------------------------

    def ref_name(self, ref: str) -> str:
        if ref.startswith("#/components/schemas/"):
            return ref.rsplit("/", 1)[1]
        return ""

    def py_type(self, schema: dict[str, Any], hint: str) -> str:
        if "$ref" in schema:
            name = self.ref_name(schema["$ref"])
            return name if name else "dict[str, Any]"
        if "enum" in schema:
            return "Literal[" + ", ".join(json.dumps(v) for v in schema["enum"]) + "]"
        kind = schema.get("type")
        if kind == "string":
            return "str"
        if kind == "integer":
            return "int"
        if kind == "number":
            return "float"
        if kind == "boolean":
            return "bool"
        if kind == "array":
            return f"list[{self.py_type(schema.get('items', {}), hint + 'Item')}]"
        if kind == "object" or "properties" in schema:
            props = schema.get("properties")
            if props:
                self.add_typed_dict(hint, schema)
                return hint
            extra = schema.get("additionalProperties")
            if isinstance(extra, dict) and extra:
                return f"dict[str, {self.py_type(extra, hint + 'Value')}]"
            return "dict[str, Any]"
        return "Any"

    def add_typed_dict(self, name: str, schema: dict[str, Any]) -> None:
        required = set(schema.get("required", []))
        props: dict[str, Any] = schema.get("properties", {})
        req_lines: list[str] = []
        opt_lines: list[str] = []
        for prop, sub in props.items():
            line = f"    {prop}: {self.py_type(sub, name + pascal(prop))}"
            (req_lines if prop in required else opt_lines).append(line)
        doc = (schema.get("description") or "").strip().split("\n")[0]
        out: list[str] = []
        if req_lines and opt_lines:
            out += [f"class _{name}Required(TypedDict):", *req_lines, "", ""]
            out += [f"class {name}(_{name}Required, total=False):"]
            out += self.doc(doc) + opt_lines
        elif req_lines:
            out += [f"class {name}(TypedDict):", *self.doc(doc), *req_lines]
        else:
            out += [f"class {name}(TypedDict, total=False):", *self.doc(doc), *opt_lines]
        self.typed_dicts[name] = out

    @staticmethod
    def doc(text: str) -> list[str]:
        return [f'    """{text.replace(chr(34), chr(39))}"""', ""] if text else []

    def build_schemas(self) -> None:
        for name, schema in self.schemas.items():
            if schema.get("type") == "object" and schema.get("properties"):
                self.add_typed_dict(name, schema)
            else:
                self.aliases[name] = self.py_type(schema, name)

    # Operations ----------------------------------------------------------

    def resolve(self, obj: dict[str, Any]) -> dict[str, Any]:
        while "$ref" in obj and obj["$ref"].startswith("#/"):
            node: Any = self.spec
            for part in obj["$ref"][2:].split("/"):
                node = node[part]
            obj = node
        return obj

    def operations(self) -> list[dict[str, Any]]:
        ops: list[dict[str, Any]] = []
        for path, item in self.spec["paths"].items():
            shared = item.get("parameters", [])
            for method in METHODS:
                op = item.get(method)
                if not op:
                    continue
                ok = {c: r for c, r in op["responses"].items() if c.startswith("2")}
                content = next(iter(ok.values())).get("content", {})
                if "text/event-stream" in content:
                    continue
                params = [self.resolve(p) for p in shared + op.get("parameters", [])]
                ops.append(
                    {
                        "id": op["operationId"],
                        "method": method.upper(),
                        "path": path,
                        "summary": (op.get("summary") or "").strip(),
                        "scope": op.get("x-required-scope", ""),
                        "params": params,
                        "body": op.get("requestBody"),
                        "ok": sorted(int(c) for c in ok),
                        "content": content,
                    }
                )
        return ops

    def response_type(self, op: dict[str, Any]) -> str:
        types: list[str] = []
        for media, body in op["content"].items():
            if media == "application/json":
                t = self.py_type(body.get("schema", {}), pascal(op["id"]) + "Response")
            elif media.startswith("text/"):
                t = "str"
            else:
                t = "bytes"
            if t not in types:
                types.append(t)
        return " | ".join(types) if types else "None"

    def body_type(self, op: dict[str, Any]) -> tuple[str, str] | None:
        body = op["body"]
        if not body:
            return None
        content = body["content"]
        media = "application/json" if "application/json" in content else next(iter(content))
        if media == "application/json":
            return media, self.py_type(
                content[media].get("schema", {}), pascal(op["id"]) + "Request"
            )
        return media, "bytes"

    def method_source(self, op: dict[str, Any]) -> list[str]:
        path_params = [p for p in op["params"] if p["in"] == "path"]
        query_params = [p for p in op["params"] if p["in"] == "query"]
        body = self.body_type(op)
        sig = ["self"]
        names: dict[str, str] = {}
        for p in path_params:
            names[p["name"]] = snake(p["name"])
            sig.append(f"{names[p['name']]}: str")
        if body:
            required = op["body"].get("required", False)
            sig.append(f"body: {body[1]}" if required else f"body: {body[1]} | None = None")
        required_query = [x for x in query_params if x.get("required")]
        optional_query = [x for x in query_params if not x.get("required")]
        for p in required_query:
            names[p["name"]] = snake(p["name"])
            t = self.py_type(p.get("schema", {}), "Query" + pascal(p["name"]))
            sig.append(f"{names[p['name']]}: {t}")
        if optional_query:
            sig.append("*")
        for p in optional_query:
            names[p["name"]] = snake(p["name"])
            t = self.py_type(p.get("schema", {}), "Query" + pascal(p["name"]))
            sig.append(f"{names[p['name']]}: {t} | None = None")
        ret = self.response_type(op)
        lines = [
            f"    def {snake(op['id'])}({', '.join(sig)}) -> {ret}:",
        ]
        doc = op["summary"] or f"Call {op['id']}."
        doc = doc.replace(chr(34), chr(39))
        lines.append(f'        """{doc} Needs scope `{op["scope"]}`."""')
        pp = "{" + ", ".join(f"{q(p['name'])}: {names[p['name']]}" for p in path_params) + "}"
        qp = "{" + ", ".join(f"{q(p['name'])}: {names[p['name']]}" for p in query_params) + "}"
        lines += [
            "        return self._call(",
            f"            {q(op['method'])},",
            f"            {q(op['path'])},",
            f"            path_params={pp},",
            f"            query={qp},",
            f"            body={'body' if body else 'None'},",
            f"            body_type={q(body[0]) if body else 'None'},",
            f"            expected={tuple(op['ok'])!r},",
            "        )",
        ]
        return lines

    # Output --------------------------------------------------------------

    def render(self, digest: str) -> str:
        self.build_schemas()
        methods: list[str] = []
        for op in self.operations():
            methods += [*self.method_source(op), ""]
        info = self.spec["info"]
        head = [
            "# SPDX-License-Identifier: GPL-3.0-only",
            "# Generated by sdk/python/scripts/generate_client.py. Do not edit by hand.",
            f"# Source: contracts/openapi/fps-api-v1.yaml, API version {info['version']},",
            f"# sha256 {digest}.",
            '"""Types and operations of the FixingPointSystem controller API."""',
            "",
            "from __future__ import annotations",
            "",
            "from typing import Any, Literal, TypedDict",
            "",
            f"API_VERSION = {q(info['version'])}",
            f"SOURCE_SHA256 = {q(digest)}",
            "",
            "",
        ]
        body: list[str] = []
        for name, expr in self.aliases.items():
            body += [f"{name} = {expr}", ""]
        body.append("")
        for name in sorted(self.typed_dicts):
            body += self.typed_dicts[name] + ["", ""]
        cls = [
            "class GeneratedOperations:",
            '    """One method per operation. `Client` supplies `_call`."""',
            "",
            "    def _call(",
            "        self,",
            "        method: str,",
            "        path: str,",
            "        *,",
            "        path_params: dict[str, str],",
            "        query: dict[str, Any],",
            "        body: Any,",
            "        body_type: str | None,",
            "        expected: tuple[int, ...],",
            "    ) -> Any:",
            "        raise NotImplementedError",
            "",
            *methods,
        ]
        text = "\n".join(head + body + cls).rstrip() + "\n"
        return text


def render(spec_path: Path) -> str:
    import yaml

    raw = spec_path.read_bytes()
    spec = yaml.safe_load(raw)
    return Generator(spec).render(hashlib.sha256(raw).hexdigest())


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--spec", type=Path, default=DEFAULT_SPEC)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--check", action="store_true", help="fail when the output is stale")
    args = parser.parse_args(argv)
    text = render(args.spec)
    if args.check:
        current = args.output.read_text(encoding="utf-8") if args.output.exists() else ""
        if current != text:
            print(
                f"{args.output} is out of date with {args.spec}. "
                "Run python3 sdk/python/scripts/generate_client.py",
                file=sys.stderr,
            )
            return 1
        return 0
    args.output.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())

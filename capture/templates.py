# SPDX-License-Identifier: GPL-3.0-only
"""Template text for generated scenarios, and a reference renderer that checks it.

``escape_literal`` turns arbitrary bytes into template text that renders back to the
same bytes under the grammar of ``contracts/README.md``: printable ASCII stays as text,
everything else becomes ``{{ hex('..') }}``, and brace pairs that would open or close a
placeholder are written through ``hex`` too.

``render`` is a small reference interpreter of the template subset that the generator
emits (``hex``, ``fill``, ``rand_int``, ``vars.x``, ``last``). The generator uses it to
check that every generated template renders to as many bytes as the captured payload,
and the tests use it to run generated scenarios without the engine.
"""

from __future__ import annotations

import random
import re
from collections.abc import Callable, Mapping
from typing import Any

# Bytes that stay literal in template text. Control characters other than TAB, LF, and
# CR, and every byte above 0x7e, go through hex so that YAML and the engine agree on them.
_PRINTABLE = frozenset(range(0x20, 0x7F)) | {0x09, 0x0A, 0x0D}
_HEX_CHUNK = 512
FILL_ALPHABET = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 .,;:-_<>/=\"'\n"


def hex_placeholder(data: bytes) -> str:
    """Return ``{{ hex('..') }}`` placeholders for ``data``, in chunks."""
    return "".join(
        "{{ hex('" + data[i : i + _HEX_CHUNK].hex() + "') }}"
        for i in range(0, len(data), _HEX_CHUNK)
    )


_BRACES = (0x7B, 0x7D)


def _needs_hex(data: bytes, i: int) -> bool:
    b = data[i]
    if b not in _PRINTABLE:
        return True
    if b not in _BRACES:  # "{" and "}"
        return False
    # A brace is literal only inside plain text. Next to a placeholder or to another brace it
    # could join with a brace of a neighbouring run once the placeholders are cut out, and the
    # validator (and a reader that looks for "{{" and "}}") would see a stray pair.
    prev = data[i - 1] if i > 0 else None
    nxt = data[i + 1] if i + 1 < len(data) else None
    return prev not in _PRINTABLE or nxt not in _PRINTABLE or prev in _BRACES or nxt in _BRACES


def escape_literal(data: bytes) -> str:
    """Return template text that renders to exactly ``data``."""
    out: list[str] = []
    i, n = 0, len(data)
    while i < n:
        j = i
        if _needs_hex(data, i):
            while j < n and _needs_hex(data, j):
                j += 1
            out.append(hex_placeholder(data[i:j]))
        else:
            while j < n and not _needs_hex(data, j):
                j += 1
            out.append(data[i:j].decode("ascii"))
        i = j
    return "".join(out)


# -- reference renderer -------------------------------------------------------------

_PLACEHOLDER = re.compile(r"\{\{(.*?)\}\}", re.S)
_TOKEN = re.compile(
    r"\s*(?:(?P<str>'[^']*')|(?P<int>-?\d+)|(?P<name>[A-Za-z_][A-Za-z0-9_.]*)|(?P<p>[(),]))"
)


class TemplateError(ValueError):
    """A template that the reference renderer cannot evaluate."""


def _tokens(text: str) -> list[tuple[str, str]]:
    pos, out = 0, []
    while pos < len(text):
        if text[pos:].strip() == "":
            break
        m = _TOKEN.match(text, pos)
        if m is None:
            raise TemplateError(f"cannot read expression {text!r} at {pos}")
        kind = m.lastgroup or ""
        out.append((kind, m.group(kind)))
        pos = m.end()
    return out


class _Evaluator:
    def __init__(
        self,
        vars_: Mapping[str, Any],
        last: bytes,
        rng: random.Random,
        fill: Callable[[int], bytes],
    ) -> None:
        self.vars, self.last, self.rng, self.fill = vars_, last, rng, fill
        self.toks: list[tuple[str, str]] = []
        self.i = 0

    def run(self, text: str) -> bytes:
        self.toks, self.i = _tokens(text), 0
        value = self.expr()
        if self.i != len(self.toks):
            raise TemplateError(f"trailing tokens in {text!r}")
        if isinstance(value, bytes):
            return value
        return str(value).encode("utf-8")

    def expr(self) -> Any:
        if self.i >= len(self.toks):
            raise TemplateError("empty expression")
        kind, tok = self.toks[self.i]
        self.i += 1
        if kind == "str":
            return tok[1:-1]
        if kind == "int":
            return int(tok)
        if kind != "name":
            raise TemplateError(f"unexpected {tok!r}")
        if tok == "last":
            return self.last
        if tok.startswith("vars."):
            name = tok[5:]
            if name not in self.vars:
                raise TemplateError(f"unknown variable {name}")
            return self.vars[name]
        if self.i < len(self.toks) and self.toks[self.i][1] == "(":
            self.i += 1
            args: list[Any] = []
            while self.toks[self.i][1] != ")":
                args.append(self.expr())
                if self.toks[self.i][1] == ",":
                    self.i += 1
            self.i += 1
            return self.call(tok, args)
        raise TemplateError(f"unknown name {tok}")

    def call(self, name: str, args: list[Any]) -> Any:
        if name == "hex" and len(args) == 1:
            return bytes.fromhex(str(args[0]))
        if name == "fill" and len(args) == 1:
            return self.fill(int(args[0]))
        if name == "rand_int" and len(args) == 2:
            return self.rng.randint(int(args[0]), int(args[1]))
        raise TemplateError(f"unsupported call {name}")


def render(
    template: str,
    vars_: Mapping[str, Any] | None = None,
    *,
    last: bytes = b"",
    rng: random.Random | None = None,
) -> bytes:
    """Render ``template`` with the reference interpreter.

    ``fill(n)`` returns ``n`` bytes from the engine's filler alphabet. ``rand_int`` draws
    from ``rng``. Text outside placeholders is copied as UTF-8.
    """
    rng = rng or random.Random(0)
    values = vars_ or {}

    def fill(n: int) -> bytes:
        return bytes(rng.choice(FILL_ALPHABET) for _ in range(n))

    out: list[bytes] = []
    pos = 0
    for m in _PLACEHOLDER.finditer(template):
        out.append(template[pos : m.start()].encode("utf-8"))
        out.append(_Evaluator(values, last, rng, fill).run(m.group(1)))
        pos = m.end()
    out.append(template[pos:].encode("utf-8"))
    return b"".join(out)

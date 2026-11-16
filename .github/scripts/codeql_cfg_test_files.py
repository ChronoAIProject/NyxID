#!/usr/bin/env python3
"""List Rust files that are reachable only under cfg(test).

The walk follows Cargo crate roots and Rust's module file rules, including
#[path] and include!. A file is listed only when every discovered reach
requires the test configuration. cfg(any(test, ...)) stays, because that file
can be built without cfg(test). An unresolvable path or include! in a
production-possible region disables exclusions for the entire tree: a macro or
include can reach across package boundaries. Filename patterns are not used.
"""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path


SKIP_DIRS = {"target", ".git", "node_modules"}
MAX_ATOMS = 12


class LexError(Exception):
    pass


@dataclass(frozen=True)
class Tok:
    kind: str
    text: str


@dataclass(frozen=True)
class Group:
    delim: str
    items: tuple


Tree = Tok | Group


def lex(source: str, origin: str) -> list[Tok]:
    """Lex Rust far enough to recover items, attributes, and include!."""

    text = source.lstrip("\ufeff")
    if text.startswith("#!") and text[2:3] not in {"", "["}:
        # A shebang is not an inner attribute.
        newline = text.find("\n")
        text = "" if newline < 0 else text[newline + 1 :]
    tokens: list[Tok] = []
    i = 0
    n = len(text)

    def peek(offset: int = 0) -> str:
        pos = i + offset
        return text[pos] if pos < n else ""

    while i < n:
        ch = text[i]
        if ch in " \t\r\n":
            i += 1
            continue
        if ch == "/" and peek(1) == "/":
            i += 2
            while i < n and text[i] != "\n":
                i += 1
            continue
        if ch == "/" and peek(1) == "*":
            i += 2
            depth = 1
            while i + 1 < n and depth:
                if text[i:i + 2] == "/*":
                    depth += 1
                    i += 2
                elif text[i:i + 2] == "*/":
                    depth -= 1
                    i += 2
                else:
                    i += 1
            if depth:
                raise LexError(f"unclosed block comment in {origin}")
            continue
        if ch == "'" and _is_lifetime(text, i):
            i += 1
            while i < n and (text[i].isalnum() or text[i] == "_"):
                i += 1
            continue
        if ch == "'":
            i = _skip_char(text, i, origin)
            tokens.append(Tok("lit", "char"))
            continue
        if ch == "#":
            tokens.append(Tok("punct", "#"))
            i += 1
            continue
        if ch in "()[]{},.;:=!<>+-*/&|^%@?~$\\":
            if ch == "." and peek(1) == "." and peek(2) == ".":
                tokens.append(Tok("punct", "..."))
                i += 3
                continue
            if ch == "." and peek(1) == ".":
                tokens.append(Tok("punct", ".."))
                i += 2
                continue
            if ch == ":" and peek(1) == ":":
                tokens.append(Tok("punct", "::"))
                i += 2
                continue
            if ch == "-" and peek(1) == ">":
                tokens.append(Tok("punct", "->"))
                i += 2
                continue
            if ch == "=" and peek(1) == ">":
                tokens.append(Tok("punct", "=>"))
                i += 2
                continue
            tokens.append(Tok("punct", ch))
            i += 1
            continue
        prefix = _string_prefix(text, i)
        if prefix is not None:
            raw, base = prefix
            i += len(base)
            if raw:
                hashes = 0
                while peek() == "#":
                    hashes += 1
                    i += 1
                if peek() != '"':
                    raise LexError(f"raw-string dispatch failed in {origin}")
            kind = "byte" if base in {"b", "br"} else "cstr" if base in {"c", "cr"} else "str"
            value, i = _scan_string(text, i, hashes if raw else 0, origin, kind, raw)
            tokens.append(Tok("str", value))
            continue
        if ch == "r" and peek(1) == "#" and not _raw_string_at(text, i):
            ident, i = _scan_raw_ident(text, i, origin)
            tokens.append(Tok("ident", ident))
            continue
        if ch.isalpha() or ch == "_":
            start = i
            i += 1
            while i < n and (text[i].isalnum() or text[i] == "_"):
                i += 1
            tokens.append(Tok("ident", text[start:i]))
            continue
        if ch.isdigit():
            i = _skip_number(text, i)
            tokens.append(Tok("lit", "num"))
            continue
        raise LexError(f"unexpected {ch!r} at {origin}:{i}")
    return tokens


def _is_lifetime(text: str, i: int) -> bool:
    nxt = text[i + 1] if i + 1 < len(text) else ""
    if not (nxt.isalpha() or nxt == "_"):
        return False
    # 'x' is a character; 'xy and 'static are lifetimes.
    end = i + 2
    while end < len(text) and (text[end].isalnum() or text[end] == "_"):
        end += 1
    return not (end == i + 2 and end < len(text) and text[end] == "'")


def _skip_char(text: str, i: int, origin: str) -> int:
    """Skip one character literal. Byte chars (`b'\\xFF'`) share this scanner."""

    i += 1
    if i >= len(text):
        raise LexError(f"unclosed character in {origin}")
    if text[i] == "\\":
        _kind, i = _consume_escape(text, i, origin, "char")
    elif text[i] == "\n" or text[i] == "\r":
        raise LexError(f"unclosed character in {origin}")
    else:
        i += 1
    if i >= len(text) or text[i] != "'":
        raise LexError(f"unclosed character in {origin}")
    return i + 1


def _raw_string_at(text: str, i: int) -> bool:
    j = i + 1
    while j < len(text) and text[j] == "#":
        j += 1
    return j < len(text) and text[j] == '"'


def _string_prefix(text: str, i: int) -> tuple[bool, str] | None:
    """Return (raw, prefix) when a string literal starts at i.

    `r#ident` is a raw identifier, not a string. A raw string requires `#`
    hashes and then a quote.
    """

    for base in ("br", "cr", "b", "c", "r"):
        if text.startswith(base, i):
            rest = i + len(base)
            if base.endswith("r"):
                if rest < len(text) and text[rest] == '"':
                    return True, base
                if rest < len(text) and text[rest] == "#":
                    j = rest
                    while j < len(text) and text[j] == "#":
                        j += 1
                    if j < len(text) and text[j] == '"':
                        return True, base
                return None
            if rest < len(text) and text[rest] == '"':
                return False, base
            return None
    if i < len(text) and text[i] == '"':
        return False, ""
    return None


def _scan_raw_ident(text: str, i: int, origin: str) -> tuple[str, int]:
    i += 2  # r#
    if i >= len(text) or not (text[i].isalpha() or text[i] == "_"):
        raise LexError(f"bad raw ident in {origin}")
    start = i
    i += 1
    while i < len(text) and (text[i].isalnum() or text[i] == "_"):
        i += 1
    return text[start:i], i


def _hex_value(text: str, i: int, count: int, origin: str) -> tuple[int, int]:
    end = i + count
    if end > len(text) or any(ch not in "0123456789abcdefABCDEF" for ch in text[i:end]):
        raise LexError(f"bad hex escape in {origin}")
    return int(text[i:end], 16), end


def _consume_escape(text: str, i: int, origin: str, kind: str) -> tuple[str | None, int]:
    """Consume a Rust string or character escape starting at the backslash.

    `kind` is `str`, `byte`, `cstr`, or `char`. Returns the decoded text
    (empty for a string continuation) and the index after the escape.
    String continuation is `\\` plus a line ending, then ASCII whitespace
    (`U+0009`, `U+000A`, `U+000D`, `U+0020`), matching the reference's
    string-continuation escape. `\\xHH` is an ASCII escape in `str` and
    `char` (00–7F) and a byte escape in byte and C strings (00–FF).
    `\\u{...}` is a Unicode escape of 1–6 hex digits.
    """

    n = len(text)
    if i >= n or text[i] != "\\":
        raise LexError(f"bad escape in {origin}")
    if i + 1 >= n:
        raise LexError(f"bad escape in {origin}")
    nxt = text[i + 1]
    simple = {
        "n": "\n",
        "r": "\r",
        "t": "\t",
        "\\": "\\",
        "'": "'",
        '"': '"',
        "0": "\0",
    }
    if nxt in simple:
        if kind == "cstr" and nxt == "0":
            raise LexError(f"nul escape in c string in {origin}")
        return simple[nxt], i + 2
    if nxt in "\n\r" and kind != "char":
        i += 2
        if nxt == "\r" and i < n and text[i] == "\n":
            i += 1
        while i < n and text[i] in " \t\n\r":
            i += 1
        return "", i
    if nxt == "x":
        value, i = _hex_value(text, i + 2, 2, origin)
        if kind in {"str", "char"} and value > 0x7F:
            # `b'\xFF'` is lexed as an identifier plus a character literal.
            # Accept the full byte so a legal byte character does not poison.
            if kind == "str":
                raise LexError(f"non-ascii hex escape in {origin}")
        if kind == "cstr" and value == 0:
            raise LexError(f"nul escape in c string in {origin}")
        return chr(value), i
    if nxt == "u" and kind in {"str", "char"}:
        if i + 2 >= n or text[i + 2] != "{":
            raise LexError(f"bad unicode escape in {origin}")
        j = i + 3
        start = j
        while j < n and text[j] != "}":
            j += 1
        digits = text[start:j]
        if not digits or len(digits) > 6 or j >= n or any(ch not in "0123456789abcdefABCDEF" for ch in digits):
            raise LexError(f"bad unicode escape in {origin}")
        value = int(digits, 16)
        if value > 0x10FFFF or 0xD800 <= value <= 0xDFFF:
            raise LexError(f"bad unicode escape in {origin}")
        return chr(value), j + 1
    shown = "newline" if nxt == "\n" else nxt
    raise LexError(f"unsupported escape \\{shown} in {origin}")


def _scan_string(
    text: str, i: int, hashes: int, origin: str, kind: str = "str", raw: bool = False
) -> tuple[str, int]:
    if i >= len(text) or text[i] != '"':
        raise LexError(f"string expected in {origin}")
    i += 1
    chars: list[str] = []
    n = len(text)
    while i < n:
        ch = text[i]
        if ch == '"' and text[i + 1 : i + 1 + hashes] == ("#" * hashes):
            return "".join(chars), i + 1 + hashes
        # Raw strings, including r"...", keep backslashes. Only cooked strings
        # interpret escapes. hashes == 0 is also the cooked-string case.
        if not raw and ch == "\\":
            decoded, i = _consume_escape(text, i, origin, kind)
            if decoded:
                chars.append(decoded)
            continue
        chars.append(ch)
        i += 1
    raise LexError(f"unclosed string in {origin}")


def _skip_number(text: str, i: int) -> int:
    n = len(text)
    if text[i] == "0" and i + 1 < n and text[i + 1] in "boxBOX":
        i += 2
        while i < n and (text[i].isalnum() or text[i] == "_"):
            i += 1
        return i
    while i < n and (text[i].isdigit() or text[i] == "_"):
        i += 1
    if i + 1 < n and text[i] == "." and text[i + 1].isdigit():
        i += 1
        while i < n and (text[i].isdigit() or text[i] == "_"):
            i += 1
    if i < n and text[i] in "eE":
        j = i + 1
        if j < n and text[j] in "+-":
            j += 1
        if j < n and text[j].isdigit():
            i = j
            while i < n and (text[i].isdigit() or text[i] == "_"):
                i += 1
    return i


def group_tokens(tokens: list[Tok], origin: str) -> list[Tree]:
    def parse(index: int, closer: str | None) -> tuple[list[Tree], int]:
        items: list[Tree] = []
        while index < len(tokens):
            tok = tokens[index]
            if tok.kind == "punct" and tok.text in ")]}":
                if tok.text != closer:
                    raise LexError(f"unbalanced {tok.text} in {origin}")
                return items, index + 1
            if tok.kind == "punct" and tok.text in "([{":
                closer_of = {"(": ")", "[": "]", "{": "}"}[tok.text]
                inner, index = parse(index + 1, closer_of)
                items.append(Group(tok.text, tuple(inner)))
                continue
            items.append(tok)
            index += 1
        if closer is not None:
            raise LexError(f"unclosed {closer} in {origin}")
        return items, index

    trees, index = parse(0, None)
    if index != len(tokens):
        raise LexError(f"trailing tokens in {origin}")
    return trees


# Formulas are tuples so they can be cached and compared.
# ("t",) ("f",) ("a", name) ("n", x) ("&", xs) ("|", xs)


def F_TRUE():
    return ("t",)


def F_FALSE():
    return ("f",)


def atom(name: str):
    return ("a", name)


def f_not(node):
    if node == ("t",):
        return ("f",)
    if node == ("f",):
        return ("t",)
    if node[0] == "n":
        return node[1]
    return ("n", node)


def f_and(nodes: list):
    flat = []
    for node in nodes:
        if node == ("f",):
            return ("f",)
        if node == ("t",):
            continue
        if node[0] == "&":
            flat.extend(node[1])
        else:
            flat.append(node)
    uniq = tuple(sorted(set(flat), key=repr))
    if not uniq:
        return ("t",)
    if len(uniq) == 1:
        return uniq[0]
    return ("&", uniq)


def f_or(nodes: list):
    flat = []
    for node in nodes:
        if node == ("t",):
            return ("t",)
        if node == ("f",):
            continue
        if node[0] == "|":
            flat.extend(node[1])
        else:
            flat.append(node)
    uniq = tuple(sorted(set(flat), key=repr))
    if not uniq:
        return ("f",)
    if len(uniq) == 1:
        return uniq[0]
    return ("|", uniq)


def atoms_of(node) -> set[str]:
    kind = node[0]
    if kind == "a":
        return {node[1]}
    if kind == "n":
        return atoms_of(node[1])
    if kind in {"&", "|"}:
        found: set[str] = set()
        for child in node[1]:
            found.update(atoms_of(child))
        return found
    return set()


def _eval(node, env: dict[str, bool]) -> bool:
    kind = node[0]
    if kind == "t":
        return True
    if kind == "f":
        return False
    if kind == "a":
        return env[node[1]]
    if kind == "n":
        return not _eval(node[1], env)
    if kind == "&":
        return all(_eval(child, env) for child in node[1])
    if kind == "|":
        return any(_eval(child, env) for child in node[1])
    raise AssertionError(kind)


_SAT: dict[tuple, bool] = {}


def satisfiable(node, test: bool) -> bool:
    key = (node, test)
    cached = _SAT.get(key)
    if cached is not None:
        return cached
    names = sorted(name for name in atoms_of(node) if name != "test")
    if len(names) > MAX_ATOMS:
        _SAT[key] = True
        return True
    for mask in range(1 << len(names)):
        env = {"test": test}
        for bit, name in enumerate(names):
            env[name] = bool(mask & (1 << bit))
        if _eval(node, env):
            _SAT[key] = True
            return True
    _SAT[key] = False
    return False


def prod_possible(node) -> bool:
    return satisfiable(node, False)


def test_possible(node) -> bool:
    return satisfiable(node, True)


KEYWORDS = {
    "fn",
    "mod",
    "impl",
    "trait",
    "use",
    "extern",
    "struct",
    "enum",
    "union",
    "type",
    "static",
    "const",
    "macro_rules",
    "let",
}


def _ident_at(trees: list[Tree] | tuple, index: int) -> str | None:
    if index < 0 or index >= len(trees):
        return None
    tree = trees[index]
    if isinstance(tree, Tok) and tree.kind == "ident":
        return tree.text
    return None


def keyword_index(trees: list[Tree] | tuple) -> int | None:
    for index, tree in enumerate(trees):
        if isinstance(tree, Tok) and tree.kind == "ident" and tree.text in KEYWORDS:
            if tree.text == "const":
                cursor = index + 1
                while cursor < len(trees):
                    qualifier = trees[cursor]
                    if not isinstance(qualifier, Tok):
                        break
                    if qualifier.kind == "ident" and qualifier.text == "fn":
                        return cursor
                    if qualifier.kind != "str" and qualifier.text not in {"unsafe", "extern"}:
                        break
                    cursor += 1
            return index
    return None


@dataclass
class Item:
    cfg: tuple
    paths: list[tuple[tuple, str | None]]
    trees: list[Tree]
    bad_path: bool = False


def _parse_expr(trees: tuple | list, index: int) -> tuple[tuple, int]:
    if index >= len(trees):
        return atom("unparsed"), index
    tree = trees[index]
    if not isinstance(tree, Tok) or tree.kind != "ident":
        return atom("unparsed"), index + 1
    name = tree.text
    nxt = trees[index + 1] if index + 1 < len(trees) else None
    if isinstance(nxt, Group) and nxt.delim == "(":
        args, _ = _parse_seq(nxt.items, 0)
        if name == "all":
            return f_and(args), index + 2
        if name == "any":
            return f_or(args), index + 2
        if name == "not":
            return f_not(args[0] if args else F_TRUE()), index + 2
        return atom(f"call:{name}"), index + 2
    if (
        isinstance(nxt, Tok)
        and nxt.kind == "punct"
        and nxt.text == "="
        and index + 2 < len(trees)
        and isinstance(trees[index + 2], Tok)
        and trees[index + 2].kind == "str"
    ):
        return atom(f"{name}:{trees[index + 2].text}"), index + 3
    if name == "test":
        return atom("test"), index + 1
    return atom(f"id:{name}"), index + 1


def _parse_seq(trees: tuple | list, index: int) -> tuple[list[tuple], int]:
    nodes = []
    if index >= len(trees):
        return nodes, index
    while index < len(trees):
        node, index = _parse_expr(trees, index)
        nodes.append(node)
        if (
            index < len(trees)
            and isinstance(trees[index], Tok)
            and trees[index].kind == "punct"
            and trees[index].text == ","
        ):
            index += 1
            continue
        break
    return nodes, index


def _meta_from_trees(trees: tuple | list) -> tuple[tuple, list[tuple[tuple, str | None]], bool]:
    """Return (added cfg, path specs, unparsed). Path spec is (when, literal or None)."""

    if not trees:
        return F_TRUE(), [], True
    first = trees[0]
    if not isinstance(first, Tok) or first.kind != "ident":
        return F_TRUE(), [], True
    if first.text == "cfg" and len(trees) >= 2 and isinstance(trees[1], Group) and trees[1].delim == "(":
        # cfg(<expr>) — the paren contents are the expression itself.
        node, _ = _parse_expr(trees[1].items, 0)
        if node == atom("unparsed"):
            return atom("unparsed-cfg"), [], True
        return node, [], False
    if first.text == "path":
        literal = _path_literal(trees)
        return F_TRUE(), [(F_TRUE(), literal)], literal is None
    if first.text == "cfg_attr" and len(trees) >= 2 and isinstance(trees[1], Group):
        return _parse_cfg_attr(trees[1].items)
    return F_TRUE(), [], False


def _path_literal(trees: tuple | list) -> str | None:
    # path = "literal"
    if (
        len(trees) >= 3
        and isinstance(trees[1], Tok)
        and trees[1].text == "="
        and isinstance(trees[2], Tok)
        and trees[2].kind == "str"
    ):
        return trees[2].text
    return None


def _parse_cfg_attr(trees: tuple | list) -> tuple[tuple, list[tuple[tuple, str | None]], bool]:
    cond, index = _parse_expr(trees, 0)
    if (
        index < len(trees)
        and isinstance(trees[index], Tok)
        and trees[index].kind == "punct"
        and trees[index].text == ","
    ):
        index += 1
    added = []
    paths = []
    bad = cond == atom("unparsed")
    while index < len(trees):
        meta_tokens = []
        while index < len(trees):
            tree = trees[index]
            if (
                isinstance(tree, Tok)
                and tree.kind == "punct"
                and tree.text == ","
            ):
                index += 1
                break
            meta_tokens.append(tree)
            index += 1
        cfg, spec, meta_bad = _meta_from_trees(meta_tokens)
        bad = bad or meta_bad
        if cfg != ("t",):
            # The inner cfg applies only when cond holds.
            added.append(f_or([f_not(cond), cfg]))
        for when, literal in spec:
            paths.append((f_and([cond, when]), literal))
    return f_and(added), paths, bad


def parse_attribute(group: Group) -> tuple[tuple, list[tuple[tuple, str | None]], bool]:
    return _meta_from_trees(group.items)


def split_items(trees: list[Tree]) -> tuple[tuple, list[Item]]:
    inner_cfgs = []
    items: list[Item] = []
    current: list[Tree] = []
    cfg_parts: list[tuple] = []
    paths: list[tuple[tuple, str | None]] = []
    bad_path = False
    started = False

    def finish() -> None:
        nonlocal current, cfg_parts, paths, bad_path
        if current or cfg_parts or paths:
            items.append(
                Item(f_and(cfg_parts), list(paths), current, bad_path)
            )
        current = []
        cfg_parts = []
        paths = []
        bad_path = False

    index = 0
    while index < len(trees):
        tree = trees[index]
        if not current and isinstance(tree, Tok) and tree.kind == "punct" and tree.text == "#":
            inner = False
            cursor = index + 1
            if cursor < len(trees) and isinstance(trees[cursor], Tok) and trees[cursor].text == "!":
                inner = True
                cursor += 1
            if cursor < len(trees) and isinstance(trees[cursor], Group) and trees[cursor].delim == "[":
                cfg, spec, bad = parse_attribute(trees[cursor])
                if inner and not started:
                    if cfg != ("t",):
                        inner_cfgs.append(cfg)
                    # Inner path attributes are not a module path.
                elif not inner:
                    if cfg != ("t",):
                        cfg_parts.append(cfg)
                    paths.extend(spec)
                    bad_path = bad_path or bad or any(literal is None for _, literal in spec)
                index = cursor + 1
                continue
        started = True
        current.append(tree)
        ends = False
        if isinstance(tree, Tok) and tree.kind == "punct" and tree.text == ";":
            ends = True
        elif isinstance(tree, Group) and tree.delim == "{":
            key_at = keyword_index(current[:-1])
            key = _ident_at(current, key_at) if key_at is not None else None
            if key in {"use", "const", "static", "type", "let"}:
                ends = False
            elif key == "extern" and _extern_is_crate(current):
                ends = False
            else:
                ends = True
        if ends:
            finish()
        index += 1
    if current or paths or cfg_parts:
        finish()
    return f_and(inner_cfgs), items


def _extern_is_crate(trees: list[Tree]) -> bool:
    key_at = keyword_index(trees)
    if key_at is None:
        return False
    return _ident_at(trees, key_at) == "extern" and _ident_at(trees, key_at + 1) == "crate"


@dataclass
class FileState:
    prod: bool = False
    test: bool = False


@dataclass
class WalkResult:
    files: dict[str, FileState]
    poisoned: dict[str, str]
    notes: list[str]

    def excluded(self) -> list[str]:
        if self.poisoned:
            return []
        # CodeQL paths-ignore is a glob syntax, so never emit a wildcard from
        # a literal Rust path. Keeping an unusual filename is conservative.
        return sorted(rel for rel, state in self.files.items()
                      if state.test and not state.prod
                      and not any(char in rel for char in "*?[]!\\\n\r"))


@dataclass
class Ctx:
    root: Path
    crate_dir: Path
    child_dir: Path
    file_dir: Path
    top_level: bool
    gate: tuple
    result: WalkResult
    stack: set[tuple[str, str]]


def analyze_tree(root: Path) -> WalkResult:
    root = root.resolve()
    result = WalkResult(files={}, poisoned={}, notes=[])
    for manifest in _manifests(root):
        try:
            data = tomllib.loads(manifest.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, tomllib.TOMLDecodeError) as exc:
            rel = _rel(root, manifest.parent)
            result.poisoned.setdefault(rel, f"unreadable manifest: {exc}")
            continue
        if "package" not in data:
            continue
        crate_dir = manifest.parent
        for source in _crate_roots(crate_dir, data):
            if not source.is_file():
                result.notes.append(f"missing crate root {source}")
                continue
            ctx = Ctx(
                root=root,
                crate_dir=crate_dir,
                child_dir=source.parent,
                file_dir=source.parent,
                top_level=True,
                gate=F_TRUE(),
                result=result,
                stack=set(),
            )
            _walk_file(source, ctx, incoming=F_TRUE(), via_path=False, crate_root=True)
    return result


def _manifests(root: Path) -> list[Path]:
    found = []
    for dirpath, dirnames, filenames in root.walk() if hasattr(Path, "walk") else _walk(root):
        dirnames[:] = [name for name in dirnames if name not in SKIP_DIRS]
        if "Cargo.toml" in filenames:
            found.append(dirpath / "Cargo.toml" if isinstance(dirpath, Path) else Path(dirpath) / "Cargo.toml")
    return sorted(found)


def _walk(root: Path):
    import os

    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [name for name in dirnames if name not in SKIP_DIRS]
        yield Path(dirpath), dirnames, filenames


def _crate_roots(crate_dir: Path, manifest: dict) -> list[Path]:
    package = manifest.get("package") or {}
    roots: list[Path] = []
    lib = manifest.get("lib")
    if isinstance(lib, dict):
        roots.append(crate_dir / str(lib.get("path") or "src/lib.rs"))
    elif (crate_dir / "src/lib.rs").is_file() and package.get("autolib", True):
        roots.append(crate_dir / "src/lib.rs")

    def auto(key: str) -> bool:
        # Edition 2018+ discovers targets even alongside explicit targets.
        # Including extra 2015 roots is conservative: it can only keep files.
        return package.get(key, True) is not False

    bins = manifest.get("bin") or []
    if not isinstance(bins, list):
        bins = []
    if auto("autobins"):
        main = crate_dir / "src/main.rs"
        if main.is_file():
            roots.append(main)
        bindir = crate_dir / "src/bin"
        if bindir.is_dir():
            for entry in sorted(bindir.iterdir()):
                if entry.suffix == ".rs" and entry.is_file():
                    roots.append(entry)
                nested = entry / "main.rs"
                if entry.is_dir() and nested.is_file():
                    roots.append(nested)
    for target in bins:
        if isinstance(target, dict):
            roots.append(_explicit_target(crate_dir, target, "src/main.rs", "src/bin"))

    for key, folder, auto_key in (
        ("test", "tests", "autotests"),
        ("example", "examples", "autoexamples"),
        ("bench", "benches", "autobenches"),
    ):
        explicit = manifest.get(key) or []
        if not isinstance(explicit, list):
            explicit = []
        if auto(auto_key):
            base = crate_dir / folder
            if base.is_dir():
                for entry in sorted(base.iterdir()):
                    if entry.suffix == ".rs" and entry.is_file():
                        roots.append(entry)
                    nested = entry / "main.rs"
                    if entry.is_dir() and nested.is_file():
                        roots.append(nested)
        for target in explicit:
            if isinstance(target, dict):
                roots.append(_explicit_target(crate_dir, target, f"{folder}/main.rs", folder))

    build = package.get("build", None)
    if build is False or build == "false":
        pass
    elif isinstance(build, str):
        roots.append(crate_dir / build)
    elif (crate_dir / "build.rs").is_file():
        roots.append(crate_dir / "build.rs")
    return sorted(set(roots))


def _explicit_target(crate_dir: Path, target: dict, fallback: str, folder: str) -> Path:
    if target.get("path"):
        return crate_dir / str(target["path"])
    name = str(target.get("name") or "")
    candidate = crate_dir / folder / f"{name}.rs"
    if candidate.is_file():
        return candidate
    nested = crate_dir / folder / name / "main.rs"
    if nested.is_file():
        return nested
    return crate_dir / fallback


def _rel(root: Path, path: Path) -> str:
    return path.resolve().relative_to(root.resolve()).as_posix()


def _poison(ctx: Ctx, reason: str) -> None:
    rel = _rel(ctx.root, ctx.crate_dir) if ctx.crate_dir != ctx.root else ""
    # Package directory relative to the source root. The root package is ".".
    key = rel or "."
    ctx.result.poisoned.setdefault(key, reason)


def _mark(ctx: Ctx, path: Path, gate: tuple) -> None:
    try:
        rel = _rel(ctx.root, path)
    except ValueError:
        if prod_possible(gate):
            ctx.result.notes.append(f"path escaped source root: {path}")
        return
    state = ctx.result.files.setdefault(rel, FileState())
    if prod_possible(gate):
        state.prod = True
    elif test_possible(gate):
        state.test = True


def _walk_file(path: Path, ctx: Ctx, incoming: tuple, via_path: bool, crate_root: bool) -> None:
    try:
        rel = _rel(ctx.root, path)
    except ValueError:
        if prod_possible(incoming):
            _poison(ctx, f"module path escapes source root: {path}")
        return
    try:
        source = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as exc:
        if prod_possible(incoming):
            _poison(ctx, f"unreadable {rel}: {exc}")
        return
    try:
        trees = group_tokens(lex(source, rel), rel)
    except LexError as exc:
        if prod_possible(incoming):
            _poison(ctx, str(exc))
        else:
            ctx.result.notes.append(str(exc))
        return
    try:
        inner, items = split_items(trees)
    except LexError as exc:
        if prod_possible(incoming):
            _poison(ctx, str(exc))
        return
    gate = f_and([incoming, inner])
    key = (rel, repr(gate))
    if key in ctx.stack:
        return
    _mark(ctx, path, gate)
    ctx.stack.add(key)
    if crate_root or via_path or path.name == "mod.rs":
        child = path.parent
    else:
        child = path.parent / path.stem
    nested = Ctx(
        root=ctx.root,
        crate_dir=ctx.crate_dir,
        child_dir=child,
        file_dir=path.parent,
        top_level=True,
        gate=gate,
        result=ctx.result,
        stack=ctx.stack,
    )
    _walk_items(items, nested)
    ctx.stack.discard(key)


def _walk_included(path: Path, ctx: Ctx, gate: tuple) -> None:
    try:
        rel = _rel(ctx.root, path)
    except ValueError:
        if prod_possible(gate):
            _poison(ctx, f"include escapes source root: {path}")
        return
    try:
        source = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as exc:
        if prod_possible(gate):
            _poison(ctx, f"unreadable include {rel}: {exc}")
        return
    try:
        trees = group_tokens(lex(source, rel), rel)
        inner, items = split_items(trees)
    except LexError as exc:
        if prod_possible(gate):
            _poison(ctx, str(exc))
        else:
            ctx.result.notes.append(str(exc))
        return
    combined = f_and([gate, inner])
    key = ("include:" + rel, repr(combined), str(ctx.child_dir), str(ctx.top_level))
    if key in ctx.stack:
        return
    _mark(ctx, path, combined)
    ctx.stack.add(key)
    nested = Ctx(
        root=ctx.root,
        crate_dir=ctx.crate_dir,
        child_dir=ctx.child_dir,
        file_dir=path.parent,
        top_level=ctx.top_level,
        gate=combined,
        result=ctx.result,
        stack=ctx.stack,
    )
    _walk_items(items, nested)
    ctx.stack.discard(key)


def _walk_items(items: list[Item], ctx: Ctx) -> None:
    for item in items:
        gate = f_and([ctx.gate, item.cfg])
        if any(
            literal is None and prod_possible(f_and([gate, when]))
            for when, literal in item.paths
        ):
            _poison(ctx, f"unresolvable path attribute near {_rel_safe(ctx)}")
        key_at = keyword_index(item.trees)
        key = _ident_at(item.trees, key_at) if key_at is not None else None
        if key == "mod" and key_at is not None:
            _walk_mod(item, ctx, gate, key_at)
            continue
        if _is_include_item(item.trees):
            _take_include(item.trees, ctx, gate)
            continue
        _scan_includes(item.trees, ctx, gate)


def _rel_safe(ctx: Ctx) -> str:
    try:
        return _rel(ctx.root, ctx.file_dir)
    except ValueError:
        return str(ctx.file_dir)


def _walk_mod(item: Item, ctx: Ctx, gate: tuple, key_at: int) -> None:
    name = _ident_at(item.trees, key_at + 1)
    if not name:
        if prod_possible(gate):
            _poison(ctx, "module declaration without a name")
        return
    body = item.trees[-1] if item.trees else None
    inline = isinstance(body, Group) and body.delim == "{"
    if inline:
        paths = [(ctx.child_dir / name,
                  f_and([gate, f_not(f_or([when for when, _ in item.paths]))]))]
        for when, literal in item.paths:
            path_gate = f_and([gate, when])
            if literal is None:
                if prod_possible(path_gate):
                    _poison(ctx, f"unresolvable inline path module {name}")
            else:
                paths.append((_path_base(ctx) / literal, path_gate))
        inner_gate, inner_items = split_items(list(body.items))
        for directory, path_gate in paths:
            if not prod_possible(path_gate) and not test_possible(path_gate):
                continue
            nested = Ctx(
                root=ctx.root, crate_dir=ctx.crate_dir, child_dir=directory,
                file_dir=ctx.file_dir, top_level=False,
                gate=f_and([path_gate, inner_gate]), result=ctx.result, stack=ctx.stack,
            )
            _walk_items(inner_items, nested)
        return
    candidates = _module_candidates(ctx, gate, name, item.paths)
    for candidate, via_path, cand_gate in candidates:
        if candidate is None:
            continue
        if not candidate.is_file():
            if prod_possible(cand_gate):
                ctx.result.notes.append(f"missing module file {candidate}")
            continue
        _walk_file(candidate, ctx, cand_gate, via_path=via_path, crate_root=False)


def _path_base(ctx: Ctx) -> Path:
    if ctx.top_level:
        return ctx.file_dir
    return ctx.child_dir


def _module_candidates(ctx: Ctx, gate: tuple, name: str, paths: list[tuple[tuple, str | None]]):
    """Yield (path, via_path_attr, gate) files this module might load."""

    default_dir = ctx.child_dir
    flat = default_dir / f"{name}.rs"
    mod_rs = default_dir / name / "mod.rs"
    if not paths:
        existing = [path for path in (flat, mod_rs) if path.is_file()]
        if len(existing) == 2:
            # rustc rejects both; keep both so neither can be dropped.
            for path in existing:
                yield path, False, gate
            return
        if len(existing) == 1:
            yield existing[0], False, gate
            return
        if prod_possible(gate):
            ctx.result.notes.append(f"missing {flat} and {mod_rs}")
        return

    names = sorted({atom_name for atom_name in atoms_of(gate)} | {
        atom_name for when, _literal in paths for atom_name in atoms_of(when)
    })
    names = [atom_name for atom_name in names if atom_name != "test"]
    if len(names) > MAX_ATOMS:
        if prod_possible(gate):
            _poison(ctx, f"too many cfg atoms on module {name}")
        return
    # A file selected with test=false is production-reachable. A file selected
    # only with test=true is cfg(test)-only. Do not reuse the parent gate for
    # the test-only file: that gate may still be satisfiable in production.
    prod_hits: dict[str, tuple[Path, bool]] = {}
    test_hits: dict[str, tuple[Path, bool]] = {}
    for test in (False, True):
        for mask in range(1 << len(names)):
            env = {"test": test}
            for bit, atom_name in enumerate(names):
                env[atom_name] = bool(mask & (1 << bit))
            if not _eval(gate, env):
                continue
            active = [(when, literal) for when, literal in paths if _eval(when, env)]
            if any(literal is None for _, literal in active):
                if not test:
                    _poison(ctx, f"unresolvable conditional path on module {name}")
                continue
            chosen: list[tuple[Path, bool]] = []
            if not active:
                existing = [path for path in (flat, mod_rs) if path.is_file()]
                if len(existing) >= 2:
                    chosen = [(path, False) for path in existing]
                elif len(existing) == 1:
                    chosen = [(existing[0], False)]
                else:
                    continue
            else:
                base = _path_base(ctx)
                chosen = [(base / literal, True) for _, literal in active]
            bucket = prod_hits if not test else test_hits
            for path, via_path in chosen:
                try:
                    rel = _rel(ctx.root, path)
                except ValueError:
                    if not test:
                        _poison(ctx, f"path escapes source root on module {name}")
                    continue
                bucket.setdefault(rel, (path, via_path))
    for rel, (path, via_path) in prod_hits.items():
        yield path, via_path, gate
    test_gate = f_and([gate, atom("test")])
    for rel, (path, via_path) in test_hits.items():
        if rel in prod_hits:
            continue
        yield path, via_path, test_gate


def _is_include_item(trees: list[Tree]) -> bool:
    if not trees:
        return False
    first = trees[0]
    return (
        isinstance(first, Tok)
        and first.kind == "ident"
        and first.text == "include"
        and len(trees) >= 3
        and isinstance(trees[1], Tok)
        and trees[1].text == "!"
        and isinstance(trees[2], Group)
    )


def _take_include(trees: list[Tree] | tuple, ctx: Ctx, gate: tuple) -> None:
    # Find include ! ( ... ) pairs anywhere in this tree, not only the first.
    seq = list(trees)
    index = 0
    while index < len(seq):
        tree = seq[index]
        if isinstance(tree, Group):
            _take_include(tree.items, ctx, gate)
            index += 1
            continue
        if (
            isinstance(tree, Tok)
            and tree.kind == "ident"
            and tree.text == "include"
            and index + 2 < len(seq)
            and isinstance(seq[index + 1], Tok)
            and seq[index + 1].text == "!"
            and isinstance(seq[index + 2], Group)
            and seq[index + 2].delim == "("
        ):
            _include_group(seq[index + 2], ctx, gate)
            index += 3
            continue
        index += 1


def _scan_includes(trees: list[Tree], ctx: Ctx, gate: tuple) -> None:
    # Modules inside blocks or macro token trees need expansion/context that
    # this walker does not resolve. Keep the package rather than overlooking a
    # production edge to a file also reached through a cfg(test) declaration.
    def has_external_module(items):
        for index, item in enumerate(items):
            if isinstance(item, Group):
                if has_external_module(item.items):
                    return True
            elif item.kind == "ident" and item.text == "mod":
                cursor = index + 1
                if cursor < len(items) and isinstance(items[cursor], Tok) and items[cursor].text == "$":
                    cursor += 1
                if _ident_at(items, cursor) is None:
                    return True
                cursor += 1
                # An inline module with only inline descendants cannot load a
                # source file, including `mod $name { ... }` in a macro.
                if cursor >= len(items) or not isinstance(items[cursor], Group) or items[cursor].delim != "{":
                    return True
        return False

    if prod_possible(gate) and has_external_module(trees):
        _poison(ctx, f"module in an unresolved block or macro in {_rel_safe(ctx)}")
    def has_include(items):
        for index, item in enumerate(items):
            if isinstance(item, Group):
                if has_include(item.items):
                    return True
            elif item.kind == "ident" and item.text == "include":
                if index + 1 < len(items) and isinstance(items[index + 1], Tok) and items[index + 1].text == "!":
                    return True
        return False

    if prod_possible(gate) and has_include(trees):
        _poison(ctx, f"include in an unresolved block or macro in {_rel_safe(ctx)}")
    _take_include(trees, ctx, gate)


def _include_group(group: Group, ctx: Ctx, gate: tuple) -> None:
    items = group.items
    if len(items) == 1 and isinstance(items[0], Tok) and items[0].kind == "str":
        target = (ctx.file_dir / items[0].text).resolve()
        _walk_included(target, ctx, gate)
        return
    if prod_possible(gate):
        _poison(ctx, f"unresolvable include! in {_rel_safe(ctx)}")


def exclusion_list(root: Path) -> list[str]:
    return analyze_tree(root).excluded()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--list", type=Path, help="Write the sorted relative paths here.")
    parser.add_argument("--json", type=Path, help="Write machine-readable stats here.")
    args = parser.parse_args(argv)
    result = analyze_tree(args.root)
    excluded = result.excluded()
    text = "".join(f"{path}\n" for path in excluded)
    if args.list:
        args.list.parent.mkdir(parents=True, exist_ok=True)
        args.list.write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)
    if args.json:
        payload = {
            "excluded": excluded,
            "excluded_count": len(excluded),
            "reached": len(result.files),
            "production": sum(1 for state in result.files.values() if state.prod),
            "test_only_reached": sum(
                1 for state in result.files.values() if state.test and not state.prod
            ),
            "poisoned": result.poisoned,
            "notes": result.notes,
        }
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

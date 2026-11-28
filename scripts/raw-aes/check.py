#!/usr/bin/env python3
"""Inventory compiler-resolved backend operations and exercise privacy in the real crate."""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

import receivers

FIXTURES = Path(__file__).resolve().parent
ROOT = FIXTURES.parent.parent
CARGO = os.environ.get("CARGO", "cargo")
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()


def cargo(root, args, env=None, compiled=None):
    command = [CARGO, *args]
    print("+", *command, flush=True)
    proc = subprocess.run(command, cwd=root, env={**os.environ, "CARGO_TARGET_DIR": str(TARGET),
                          **(env or {})}, stdout=subprocess.PIPE, text=True)
    messages = []
    for line in proc.stdout.splitlines():
        event = json.loads(line)
        backend = Path(event.get("manifest_path", "")) == root.resolve() / "backend/Cargo.toml"
        if compiled is not None and backend and event["reason"] == "compiler-artifact":
            compiled.add(Path(event["target"]["src_path"]).resolve())
            for output in map(Path, event["filenames"]):
                if output.suffix == ".rmeta":
                    dep_info = output.with_name(output.stem.removeprefix("lib") + ".d")
                    for rule in dep_info.read_text().splitlines():
                        if rule.endswith(".rs:"):
                            compiled.add((root / rule[:-1].replace("\\ ", " ")).resolve())
        if event["reason"] == "compiler-message":
            message = event["message"]
            if backend or message["level"] == "error":
                messages.append(message)
    return proc.returncode, messages


def inventory(root, conf, mode, options, compiled=None):
    policy = ("disallowed-methods = " + json.dumps(sorted(receivers.METHODS))
              + "\ndisallowed-types = " + json.dumps(sorted(receivers.TYPES)) + "\n")
    (conf / "clippy.toml").write_text(policy)
    # Clippy configuration participates even when an earlier check cached the target.
    stamp = hashlib.sha256(policy.encode()).hexdigest()[:16]
    code, messages = cargo(root, ["clippy", "--no-deps", "--locked", "-p", "nyxid", *options,
                           "--message-format=json", "--", "--cfg", "aes_inventory_" + stamp,
                           "--force-warn=clippy::disallowed_methods",
                           "--force-warn=clippy::disallowed_types"], {"CLIPPY_CONF_DIR": str(conf)}, compiled)
    uses = set()
    for msg in messages:
        if (msg["code"] or {}).get("code") in ("clippy::disallowed_methods", "clippy::disallowed_types"):
            definition = re.fullmatch(r"use of a disallowed (?:method|type) `([^`]+)`", msg["message"])[1]
            for span in msg["spans"]:
                if span["is_primary"]:
                    uses.add((span["file_name"], span["line_start"], span["column_start"], definition))
        elif msg["level"] == "error":
            print(msg["rendered"], file=sys.stderr)
    if code or not uses:
        raise RuntimeError(f"{mode}: incomplete compiler inventory (exit {code}, {len(uses)} sites)")
    blocked = set()
    for file, line, col, definition in sorted(uses):
        source = receivers.reviewed_source(file, definition)
        allowed = receivers.is_reviewed(source, definition)
        print(f"{mode}\t{'reviewed' if allowed else 'unreviewed'}\t{file}:{line}:{col}\t{definition}")
        if not allowed:
            blocked.add((file, line, definition))
    return uses, blocked


def check_inventory(root, conf):
    # Cargo discovers unit, integration, example and bench targets and expands their macros.
    known = set()
    compiled = set()
    for mode, options in [("normal", ["--bins"]), ("default-targets", ["--all-targets"]),
                          ("kms-targets", ["--all-features", "--all-targets"])]:
        uses, blocked = inventory(root, conf, mode, options, compiled)
        if blocked:
            raise RuntimeError(f"{len(blocked)} unreviewed {mode} uses")
        known.update((receivers.reviewed_source(file, op), op) for file, _, _, op in uses)
    candidates = re.compile(r"\b(?:EncryptionKeys|Aes\d+\w*|aes_gcm|encrypt|decrypt|rewrap|observe|(?:open|seal)_census_record|\w+_with_aad)\b")
    for file in (root / "backend").rglob("*.rs"):
        if file.resolve() not in compiled and candidates.search(file.read_text()):
            raise RuntimeError(f"unresolved AES candidate outside the compiled targets: {file}")
    print(f"Checked {len(compiled)} Rust sources reached by Cargo targets")
    missing = {(file, op) for file, ops in receivers.REVIEWED.items() for op in ops} - known
    if missing:
        raise RuntimeError(f"reviewed operations missing from inventory: {sorted(missing)}")


def copy_sources(root):
    # Copy the tracked working files, including local edits, without any VCS or build state.
    files = subprocess.check_output(["git", "ls-files", "--cached", "-z"], cwd=ROOT)
    for raw in files.split(b"\0"):
        if not raw:
            continue
        file = Path(os.fsdecode(raw))
        source = ROOT / file
        if source.is_file():
            target = root / file
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target, follow_symlinks=False)


def append(path, text):
    with path.open("a") as output:
        output.write("\n" + text + "\n")


class MissingPrivacyErrors(RuntimeError):
    def __init__(self, missing):
        super().__init__(f"missing privacy diagnostics: {sorted(missing)}")
        self.missing = missing


# The census APIs serve two disjoint Rust modules. In the negative build, have
# rustc mark uses of each resolved definition, including imports and re-exports.
CENSUS_DEFINITIONS = {
    "observe": ("record_field/authenticated", receivers.OBSERVE),
    "seal_census_record": ("aes/record", receivers.KEYS + "::seal_census_record"),
    "open_census_record": ("aes/record", receivers.KEYS + "::open_census_record"),
}


@contextmanager
def census_privacy_probe(root):
    originals = {}
    try:
        for name, (module, definition) in CENSUS_DEFINITIONS.items():
            file = root / f"backend/src/crypto/{module}.rs"
            text = file.read_text()
            originals.setdefault(file, text)
            declaration = f"pub(crate) async fn {name}("
            if text.count(declaration) != 1:
                raise RuntimeError(f"could not identify census declaration {module}::{name}")
            marker = f'#[deprecated(note = "raw-aes-probe:{definition}")]'
            file.write_text(text.replace(declaration, marker + "\n" + declaration))
        yield
    finally:
        for file, text in originals.items():
            file.write_text(text)


def check_privacy(root, mode, expected):
    target = ["--tests"] if mode == "test" else ["--bin", "nyxid-server"]
    options = ["--locked", "-p", "nyxid", *target, "--message-format=json"]
    # Force the marker even in modules with existing allow(deprecated) attributes.
    command = (["clippy", "--no-deps", *options, "--", "--force-warn=deprecated"]
               if expected else ["check", *options])
    code, messages = cargo(root, command)
    seen = set()
    census = set()
    for msg in messages:
        error = (msg["code"] or {}).get("code")
        definition = msg["message"].partition(": raw-aes-probe:")[2] if error == "deprecated" else None
        if definition:
            # Only a marker attached to one of our three definitions can identify a census use.
            name = definition.rsplit("::", 1)[-1]
            if not expected or definition not in receivers.CENSUS_CALLERS:
                raise RuntimeError(f"unexpected compiler error: {msg['rendered']}")
            declarations = {name}
        elif msg["level"] == "error":
            declarations = {
                name for diagnostic in [msg, *msg["children"]] for span in diagnostic["spans"]
                if span["file_name"] == "backend/src/crypto/aes.rs"
                for text in span["text"] for name in re.findall(r"\bfn (\w+)\(", text["text"])
                if f"`{name}`" in msg["message"]
            }
        else:
            continue
        spans = [span for span in msg["spans"] if span["is_primary"]]
        if not spans:
            raise RuntimeError(f"unexpected compiler error: {msg['rendered']}")
        for span in spans:
            file, line = span["file_name"], span["line_start"]
            rows = {(line, error, name) for name in declarations} & expected
            if file == f"backend/src/services/raw_aes_{mode}.rs" and rows:
                seen.update(rows)
                continue
            if definition:
                source = receivers.reviewed_source(file, definition)
                if receivers.is_reviewed(source, definition):
                    if source == receivers.CENSUS_CALLERS[definition] + ".rs":
                        census.add(name)
                    print(f"{mode}: reviewed {definition} at {file}:{line}")
                    continue
            raise RuntimeError(f"unexpected compiler error: {msg['rendered']}")
    if expected and census != CENSUS_DEFINITIONS.keys():
        raise RuntimeError(f"{mode}: missing census adapter diagnostics: {CENSUS_DEFINITIONS.keys() - census}")
    if code and not seen:
        raise RuntimeError(f"{mode}: cargo failed without an expected diagnostic (exit {code})")
    if expected - seen:
        raise MissingPrivacyErrors(expected - seen)
    if bool(code) != bool(seen):
        raise RuntimeError(f"{mode}: unexpected compiler exit {code}")
    print(f"{mode}: verified {len(seen)} privacy diagnostics", flush=True)


def check_constructor(root, conf):
    source = "backend/src/services/attachment_extraction.rs"
    file = root / source
    original = file.read_text()
    fixture = (FIXTURES / "constructor.rs").read_text()
    append(file, fixture)
    sites, blocked = inventory(root, conf, "constructor-control", ["--bin", "nyxid-server"])
    file.write_text(original)
    expected = set()
    for line, code in enumerate(fixture.splitlines(), original.count("\n") + 2):
        if "// " in code:
            name = code.split("// ")[1]
            expected.add((source, line, name))
    actual = {(path, line, name) for path, line, _, name in sites}
    if not expected <= actual or {site for site in expected if site[2] != receivers.PDF} - blocked:
        raise RuntimeError(f"constructor control missed a reviewed or forbidden operation: {expected - actual}")
    if any(site[2] == receivers.PDF for site in blocked):
        raise RuntimeError("PDF positive control was refused")


def privacy(root, conf):
    aes = root / "backend/src/crypto/aes.rs"
    services = root / "backend/src/services"
    append(aes, 'mod boundary_control {\n' + (FIXTURES / "codec.rs").read_text() + '}')
    fixture = (FIXTURES / "denied.rs").read_text()
    for mode, cfg in [("normal", "not(test)"), ("test", "test")]:
        path = services / f"raw_aes_{mode}.rs"
        append(services / "mod.rs", f"#[cfg({cfg})]\nmod raw_aes_{mode};")
        shutil.copyfile(FIXTURES / "permitted.rs", path)
        check_privacy(root, mode, set())
        path.write_text(fixture)
        expected = set()
        for n, line in enumerate(fixture.splitlines(), 1):
            mark = re.search(r"// (?:(test|normal) )?(E\d{4}|deprecated) (\w+)$", line)
            if mark and mark[1] in (None, mode):
                expected.add((n, mark[2], mark[3]))
        with census_privacy_probe(root):
            check_privacy(root, mode, expected)
            if mode == "normal":
                original = aes.read_text()
                declaration = "pub(super) async fn decrypt("
                if original.count(declaration) != 1:
                    raise RuntimeError("could not widen exactly EncryptionKeys::decrypt")
                aes.write_text(original.replace(declaration, "pub(crate) async fn decrypt("))
                try:
                    check_privacy(root, mode, expected)
                except MissingPrivacyErrors as error:
                    if error.missing != {row for row in expected if row[1:] == ("E0624", "decrypt")}:
                        raise
                    print(f"Deliberately widened EncryptionKeys::decrypt rejected: {error}", flush=True)
                else:
                    raise RuntimeError("privacy check accepted a crate-visible raw decryptor")
        shutil.copyfile(FIXTURES / "permitted.rs", path)
        if mode == "normal":
            check_constructor(root, conf)
            aes.write_text(original)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("check", choices=["inventory", "privacy", "all"], default="all", nargs="?")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="nyxid-raw-aes-") as tmp:
        tmp = Path(tmp)
        if args.check != "privacy":
            check_inventory(ROOT, tmp)
        if args.check != "inventory":
            root = tmp / "source"
            copy_sources(root)
            privacy(root, tmp)


if __name__ == "__main__":
    main()

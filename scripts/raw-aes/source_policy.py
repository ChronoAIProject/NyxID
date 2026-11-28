"""Configuration-independent checks that need no Cargo build on a PR."""
import re
import sys
from pathlib import Path

import receivers

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / ".github/scripts"))
from codeql_cfg_test_files import Group, Tok, group_tokens, lex

CIPHERS = {definition.rsplit("::", 1)[1]: definition for definition in receivers.TYPES}
SEALERS = {name.rsplit("::", 1)[1]: name for name in receivers.CENSUS_RECORDS}
# Outside the observer module, allow named evidence types without importing
# the module itself, a wildcard, or its observe function under an alias.
EVIDENCE = {"Binding", "Evidence", "Format", "LogicalKey", "Route", "Slot"}
NAMES = set(CIPHERS) | set(SEALERS) | {"aes_gcm", "cbc", "authenticated", "unwrap_for_observation"}
CANDIDATE = re.compile(r"\b(?:" + "|".join(sorted(NAMES)) + r")\b")


def tokens(text, file):
    return [t.text if t.kind in ("ident", "punct") else "" for t in lex(text, file)]


def needs_single_provider_builds(root):
    # Positive feature gates are included by --all-features. An exclusion or
    # an alternative module can select different code with just one provider.
    def alternative(items):
        for i, item in enumerate(items):
            if not isinstance(item, Group):
                continue
            head = items[i - 1] if i else None
            if head in (Tok("ident", "not"), Tok("ident", "cfg_attr")):
                if contains_feature(item.items):
                    return True
            if alternative(item.items):
                return True
        return False

    def contains_feature(items):
        return any(contains_feature(t.items) if isinstance(t, Group) else t == Tok("ident", "feature")
                   for t in items)

    for path in (root / "backend").rglob("*.rs"):
        text = path.read_text()
        if "CARGO_FEATURE_" in text or re.search(r"\bcfg_if\b", text):
            return True
        if re.search(r"\bfeature\b", text) and alternative(group_tokens(lex(text, str(path)), str(path))):
            return True
    return False


def allows_evidence(items):
    if items[:1] != ["{"]:
        return bool(items) and items[0] in EVIDENCE
    i = 1
    while i < len(items) and items[i] in EVIDENCE:
        i += 1
        if items[i:i + 1] == ["as"]:
            i += 2  # A renamed evidence type has the same definition.
        if items[i:i + 1] == [","]:
            i += 1
        elif items[i:i + 1] != ["}"]:
            return False
    return items[i:i + 1] == ["}"]


def violations(file, text):
    if not CANDIDATE.search(text):
        return set()
    stream = tokens(text, file)
    blocked = set()
    for i, name in enumerate(stream):
        if name not in NAMES:
            continue
        op = SEALERS.get(name) or CIPHERS.get(name)
        if name == "aes_gcm":
            op = "aes_gcm::Aes256Gcm"
        elif name == "cbc":
            op = "cbc::Encryptor"
        elif name == "unwrap_for_observation":
            if i and stream[i - 1] == "fn" and file.startswith("backend/src/crypto/"):
                continue
            op = receivers.OBSERVATION_UNWRAP
        elif name == "authenticated":
            after = stream[i + 1:i + 2]
            if after == ["::"] and allows_evidence(stream[i + 2:]):
                continue
            if after not in (["::"], ["as"]) and stream[max(0, i - 2):i] != ["record_field", "::"]:
                continue
            op = receivers.OBSERVE
        if not receivers.is_reviewed(receivers.reviewed_source(file, op), op):
            blocked.add(name)
    return blocked


def check_visibility(aes):
    # Verify all eight actual declarations, including the cfg(test)-only v0 pair.
    stream = tokens(aes, "backend/src/crypto/aes.rs")
    for name in receivers.RAW:
        matches = [i for i in range(len(stream) - 3) if stream[i:i + 4] == ["async", "fn", name, "("]]
        if len(matches) != 1:
            raise RuntimeError(f"expected one raw EncryptionKeys::{name} declaration")
        i = matches[0]
        if name == "rewrap":
            valid = stream[i - 1] in ("]", "}")
        else:
            valid = stream[i - 4:i] == ["pub", "(", "super", ")"]
        if not valid:
            raise RuntimeError(f"raw EncryptionKeys::{name} visibility changed")
    for name in ("encrypt", "decrypt"):
        heads = [i for i in range(1, len(stream) - 2) if stream[i:i + 3] == ["fn", name, "("]
                 and stream[i - 1] != "async"]
        if len(heads) != 1 or stream[heads[0] - 7:heads[0]] != ["#", "[", "cfg", "(", "test", ")", "]"]:
            raise RuntimeError(f"raw v0 {name} must be private and test-only")


def controls():
    pdf = "backend/src/services/attachment_extraction.rs"
    observer = "backend/src/services/encrypted_record_migration/observation/nested.rs"
    ledger = "backend/src/services/encrypted_record_census/report/store.rs"
    for file, code, denied in [
        (pdf, "let _ = lopdf::Document::encrypt;", set()),
        (pdf, '/* Aes256Gcm */ let _ = r#"authenticated::observe"#;', set()),
        (pdf, "use aes_gcm::{Aes256Gcm as C, KeyInit};", {"aes_gcm", "Aes256Gcm"}),
        (pdf, "use aes_gcm as cipher; use cipher::*;", {"aes_gcm"}),
        (pdf, "use cbc as cipher; use cipher::*;", {"cbc"}),
        ("backend/src/services/channel_adapters/lark.rs", "use aes_gcm as cipher;", {"aes_gcm"}),
        (pdf, "pub(crate) use rf::authenticated as api;", {"authenticated"}),
        (pdf, "use rf::authenticated::{Route, observe as read};", {"authenticated"}),
        (pdf, "use rf::authenticated::*;", {"authenticated"}),
        (observer, "let _ = authenticated::observe;", set()),
        (ledger, "let _ = authenticated::observe;", {"authenticated"}),
        (ledger, "use rf::authenticated::{Route as KeyRoute, Slot};", set()),
        (ledger, "let _ = Keys::seal_census_record;", set()),
        (observer, "macro_rules! read { () => { K::r#open_census_record } }", {"open_census_record"}),
    ]:
        got = violations(file, code)
        if got != denied:
            raise RuntimeError(f"source fixture {file} {code!r}: expected {denied}, got {got}")


def check(root):
    controls()
    check_visibility((root / "backend/src/crypto/aes.rs").read_text())
    files = sorted((root / "backend").rglob("*.rs"))
    failures = []
    for path in files:
        file = path.relative_to(root).as_posix()
        for item in sorted(violations(file, path.read_text())):
            failures.append(f"{file}: unreviewed {item}")
    if failures:
        raise RuntimeError("\n".join(failures))
    print(f"Source policy passed for {len(files)} Rust files, eight declarations and caller fixtures", flush=True)

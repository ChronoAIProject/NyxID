#!/usr/bin/env python3
"""Record and require real locked Cargo completion behind pinned cargo-dist.

cargo-dist 0.30 consumes CompilerArtifact messages but does not wait for the
Cargo child's successful completion. Its supported CARGO override points here.
The producer separately validates these receipts and freshly staged archives.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid


def file_hash(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(65536), b""):
            digest.update(block)
    return digest.hexdigest()


def option(arguments, name):
    values = [item.split("=", 1)[1] for item in arguments if item.startswith(name + "=")]
    values += [arguments[index + 1] for index, item in enumerate(arguments[:-1]) if item == name]
    if len(values) != 1:
        raise ValueError("required Cargo build option is missing or duplicated: " + name)
    return values[0]


def invoke(arguments, env):
    cargo = env["NYXID_DIST_REAL_CARGO"]
    child_env = {**env, "CARGO": cargo}
    if arguments[:2] != ["auditable", "build"]:
        if arguments[:1] == ["metadata"]:
            arguments = [*arguments, "--locked"]
        elif arguments not in (["-vV"], ["--version"], ["-V"], ["auditable", "--version"]):
            raise ValueError("unsupported non-build Cargo command in dist producer")
        return subprocess.call([cargo, *arguments], env=child_env)
    target = option(arguments, "--target")
    if target not in json.loads(env["NYXID_DIST_TARGETS"]) or \
            option(arguments, "--package") != "nyxid-cli" or \
            option(arguments, "--profile") != "dist" or \
            option(arguments, "--message-format") != "json-render-diagnostics" or \
            "--workspace" in arguments or "--frozen" in arguments or "--offline" in arguments:
        raise ValueError("nested Cargo command differs from validated dist build policy")
    command = [cargo, *arguments, "--locked"]
    directory = Path(env["NYXID_DIST_CARGO_RECEIPTS"])
    record = directory / (str(uuid.uuid4()) + ".json")
    identity = {"commit": env["NYXID_GIT_HASH"], "epoch": int(env["SOURCE_DATE_EPOCH"])}
    receipt = {"schema": "nyxid.dist-cargo-build.v1", "source": identity, "target": target,
               "command": command, "exit_code": None, "build_finished": [], "artifacts": []}
    # Persist an incomplete receipt before spawn; a killed wrapper cannot leave
    # an earlier success standing in for this required invocation.
    with record.open("x") as stream:
        json.dump(receipt, stream)
    messages = directory / (record.stem + ".jsonl")
    finished, artifacts = [], []
    with messages.open("xb") as log:
        process = subprocess.Popen(command, env=child_env, stdout=subprocess.PIPE)
        try:
            for line in process.stdout:
                log.write(line)
                sys.stdout.buffer.write(line)
                sys.stdout.buffer.flush()
                try:
                    event = json.loads(line)
                except (ValueError, UnicodeDecodeError):
                    continue  # Cargo diagnostics remain in the retained stream.
                if event.get("reason") == "build-finished":
                    finished.append(event.get("success"))
                if event.get("reason") == "compiler-artifact" and \
                        event.get("target", {}).get("name") == "nyxid" and \
                        event.get("target", {}).get("kind") == ["bin"] and event.get("executable"):
                    path = Path(event["executable"])
                    if path.is_symlink() or not path.is_file():
                        raise ValueError("required nested Cargo executable is missing or nonregular")
                    artifacts.append({"package_id": event["package_id"], "name": "nyxid",
                                      "path": str(path), "sha256": file_hash(path)})
        finally:
            process.stdout.close()
            code = process.wait()
    receipt.update(exit_code=code, build_finished=finished, artifacts=artifacts,
                   messages={"name": messages.name, "sha256": file_hash(messages)})
    temporary = record.with_suffix(".finished")
    with temporary.open("x") as stream:
        json.dump(receipt, stream, indent=2)
        stream.write("\n")
    temporary.replace(record)
    return 0 if code == 0 and finished == [True] and len(artifacts) == 1 else (code if code > 0 else 1)


if __name__ == "__main__":
    try:
        raise SystemExit(invoke(sys.argv[1:], os.environ.copy()))
    except (KeyError, ValueError, OSError) as error:
        print("Required dist Cargo invocation failed: " + str(error), file=sys.stderr)
        raise SystemExit(1)

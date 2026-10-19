#!/usr/bin/env python3
"""Validate a clean checkout and export exact committed inputs for a build.

Python 3.8 compatible. The source tar and receipt remain outside the exported
tree. Environment variables alone never certify a Git working-tree build.
"""
import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile


def git(root, *arguments):
    return subprocess.check_output(git_command(root, *arguments), env=git_environment())


def git_environment():
    # The caller's alternate index/worktree/object replacement/config cannot
    # change which committed objects the official export contains.
    return {**{key: value for key, value in os.environ.items() if not key.startswith("GIT_")},
            "GIT_OPTIONAL_LOCKS": "0", "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": os.devnull}


def git_command(root, *arguments):
    return ["git", "--no-replace-objects", "-C", str(root), *arguments]


def has_git_tree(tree):
    result = subprocess.run(git_command(tree, "rev-parse", "--show-toplevel"),
                            env=git_environment(), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    # An export must not inherit any Git working tree from its parent.
    return result.returncode == 0


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def within(path, root):
    try:
        path.relative_to(root)
        return True
    except ValueError:
        return False


def clean_identity(root, expected):
    if Path(git(root, "rev-parse", "--show-toplevel").decode().strip()).resolve() != root:
        raise ValueError("source root must be the checkout root")
    commit = git(root, "rev-parse", "HEAD").decode().strip()
    if not re.fullmatch(r"[0-9a-f]{40}", expected) or commit != expected:
        raise ValueError("source checkout differs from expected full commit")
    if git(root, "status", "--porcelain=v1", "-z", "--untracked-files=all"):
        raise ValueError("official source export requires a clean checkout")
    if any(line.startswith(b"160000 ") for line in git(root, "ls-tree", "-r", commit).splitlines()):
        raise ValueError("submodule source export requires an explicit authenticated mapping")
    epoch = git(root, "show", "-s", "--format=%ct", commit).decode().strip()
    if not re.fullmatch(r"0|[1-9][0-9]*", epoch):
        raise ValueError("invalid commit epoch")
    tree = git(root, "rev-parse", commit + "^{tree}").decode().strip()
    if not re.fullmatch(r"[0-9a-f]{40}", tree):
        raise ValueError("unsupported source tree identity")
    return {"commit": commit, "epoch": int(epoch), "tree": tree}


def relative(value):
    path = PurePosixPath(value)
    if not value or path.is_absolute() or str(path) != value or \
            any(part in ("", ".", "..", ".git") for part in path.parts) or \
            "\\" in value or any(ord(char) < 32 for char in value):
        raise ValueError("invalid exported source path")
    return path


def inventory(tree, generated=()):
    if tree.is_symlink() or not tree.is_dir():
        raise ValueError("source export must be a regular directory")
    excluded = [relative(value) for value in generated]
    records = []
    for parent, directories, files in os.walk(tree, followlinks=False):
        for name in sorted(directories + files):
            path = Path(parent) / name
            name = path.relative_to(tree).as_posix()
            key = relative(name)
            if any(key == value or value in key.parents for value in excluded):
                if path.name in directories:
                    directories.remove(path.name)
                continue
            if path.is_symlink():
                target = os.readlink(path)
                if PurePosixPath(target).is_absolute() or "\\" in target or \
                        any(ord(char) < 32 for char in target) or \
                        not within(path.resolve(), tree.resolve()):
                    raise ValueError("source symlink escapes export")
                records.append({"path": name, "kind": "symlink", "target": target})
            elif path.is_file():
                records.append({"path": name, "kind": "file", "size": path.stat().st_size,
                                "executable": bool(path.stat().st_mode & 0o111), "sha256": digest(path)})
            elif path.is_dir():
                records.append({"path": name, "kind": "directory"})
            else:
                raise ValueError("unsupported exported source entry")
    return sorted(records, key=lambda item: item["path"])


def committed_files(root, commit):
    records = []
    for item in git(root, "ls-tree", "-r", "-z", commit).split(b"\0"):
        if not item:
            continue
        metadata, name = item.split(b"\t", 1)
        mode, kind, oid = metadata.decode("ascii").split()
        name = str(relative(name.decode("utf-8")))
        if kind != "blob" or mode not in ("100644", "100755", "120000") or \
                not re.fullmatch(r"[0-9a-f]{40}", oid):
            raise ValueError("unsupported committed source entry")
        records.append({"path": name, "mode": mode, "git_oid": oid})
    return sorted(records, key=lambda item: item["path"])


def require_committed_tree(entries, source, commit_object):
    raw = base64.b64decode(commit_object, validate=True)
    if hashlib.sha1(b"commit " + str(len(raw)).encode() + b"\0" + raw).hexdigest() != source["commit"]:
        raise ValueError("source commit object differs from expected commit")
    header = raw.split(b"\n\n", 1)[0]
    dates = re.findall(rb"^committer .* ([0-9]+) [+-][0-9]{4}$", header, re.MULTILINE)
    if not raw.startswith(("tree " + source["tree"] + "\n").encode()) or \
            len(dates) != 1 or int(dates[0]) != source["epoch"]:
        raise ValueError("source tree/epoch differ from committed identity")
    tree = {}
    paths = set()
    for entry in entries:
        if set(entry) != {"path", "mode", "git_oid"} or \
                entry["mode"] not in ("100644", "100755", "120000") or \
                not re.fullmatch(r"[0-9a-f]{40}", entry["git_oid"]):
            raise ValueError("invalid committed source entry")
        path = relative(entry["path"])
        if path in paths:
            raise ValueError("duplicate committed source path")
        paths.add(path)
        parent = tree
        for name in path.parts[:-1]:
            parent = parent.setdefault(name, {})
            if not isinstance(parent, dict):
                raise ValueError("committed source has a non-directory ancestor")
        if path.name in parent:
            raise ValueError("committed source path collision")
        parent[path.name] = (entry["mode"], entry["git_oid"])

    def tree_oid(items):
        content = bytearray()
        for name, value in sorted(items.items(), key=lambda item:
                                  (item[0] + ("/" if isinstance(item[1], dict) else "")).encode("utf-8")):
            mode, oid = ("40000", tree_oid(value)) if isinstance(value, dict) else value
            content.extend(mode.encode() + b" " + name.encode("utf-8") + b"\0" + bytes.fromhex(oid))
        return hashlib.sha1(b"tree " + str(len(content)).encode() + b"\0" + content).hexdigest()

    if tree_oid(tree) != source["tree"]:
        raise ValueError("exported source inventory differs from committed tree")


def require_archive_tree(archive, entries):
    expected = {entry["path"]: entry for entry in entries}
    found = set()
    with tarfile.open(archive) as tar:
        for item in tar:
            name = str(relative(item.name))
            entry = expected.get(name)
            if name in found or entry is None or \
                    (item.issym() != (entry["mode"] == "120000")) or \
                    not (item.issym() or item.isfile()):
                raise ValueError("source archive differs from committed member set")
            found.add(name)
            if item.issym():
                content = item.linkname.encode("utf-8")
            else:
                if item.size > 1024 * 1024 * 1024:
                    raise ValueError("source archive member exceeds bound")
                content = tar.extractfile(item).read()
                if bool(item.mode & 0o111) != (entry["mode"] == "100755"):
                    raise ValueError("source archive differs from committed mode")
            if hashlib.sha1(b"blob " + str(len(content)).encode() + b"\0" + content).hexdigest() != entry["git_oid"]:
                raise ValueError("source archive differs from committed blob")
    if found != set(expected):
        raise ValueError("source archive omits committed input")


def require_exported_files(tree, entries, generated=()):
    actual = inventory(tree, generated)
    expected_paths = {entry["path"] for entry in entries}
    expected_dirs = {str(parent) for entry in entries for parent in relative(entry["path"]).parents
                     if str(parent) != "."}
    if {item["path"] for item in actual if item["kind"] == "directory"} != expected_dirs or \
            {item["path"] for item in actual if item["kind"] != "directory"} != expected_paths:
        raise ValueError("exported source has missing/extra committed paths")
    for entry in entries:
        path = tree / entry["path"]
        if entry["mode"] == "120000":
            if not path.is_symlink():
                raise ValueError("exported source differs from committed symlink")
            content = os.readlink(path).encode("utf-8")
        else:
            if path.is_symlink() or not path.is_file() or \
                    bool(path.stat().st_mode & 0o111) != (entry["mode"] == "100755"):
                raise ValueError("exported source differs from committed mode")
            if path.stat().st_size > 1024 * 1024 * 1024:
                raise ValueError("exported source member exceeds bound")
            content = path.read_bytes()
        if hashlib.sha1(b"blob " + str(len(content)).encode() + b"\0" + content).hexdigest() != entry["git_oid"]:
            raise ValueError("exported source differs from committed blob")
    return actual


def write_archive(root, archive, entries, epoch):
    # Read raw committed blobs, deliberately applying no export-ignore,
    # export-subst, working-tree filters, or local/global attributes. The exact
    # tree is the build input; transformations require a new explicit protocol.
    with subprocess.Popen(git_command(root, "cat-file", "--batch"),
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          stderr=subprocess.DEVNULL, env=git_environment()) as process:
        try:
            with tarfile.open(archive, "x", format=tarfile.PAX_FORMAT) as tar:
                for entry in entries:
                    process.stdin.write((entry["git_oid"] + "\n").encode("ascii"))
                    process.stdin.flush()
                    header = process.stdout.readline().decode("ascii").split()
                    if len(header) != 3 or header[:2] != [entry["git_oid"], "blob"]:
                        raise ValueError("committed source object is missing")
                    size = int(header[2])
                    if size < 0 or size > 1024 * 1024 * 1024:
                        raise ValueError("committed source object exceeds bound")
                    content = process.stdout.read(size)
                    if len(content) != size or process.stdout.read(1) != b"\n" or \
                            hashlib.sha1(b"blob " + str(size).encode() + b"\0" + content).hexdigest() != entry["git_oid"]:
                        raise ValueError("committed source object differs from tree")
                    info = tarfile.TarInfo(entry["path"])
                    info.mtime = epoch
                    info.mode = 0o755 if entry["mode"] == "100755" else 0o644
                    if entry["mode"] == "120000":
                        info.type = tarfile.SYMTYPE
                        info.mode = 0o777
                        info.linkname = content.decode("utf-8")
                        tar.addfile(info)
                    else:
                        info.size = size
                        tar.addfile(info, io.BytesIO(content))
            process.stdin.close()
            if process.wait() != 0:
                raise ValueError("could not read committed source objects")
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()


def extract_archive(archive, tree):
    with tarfile.open(archive) as tar:
        members = tar.getmembers()
        names = [relative(item.name.rstrip("/")) for item in members]
        if len(names) != len(set(names)):
            raise ValueError("duplicate source archive member")
        links = {name for name, item in zip(names, members) if item.issym()}
        for path, item in zip(names, members):
            if any(parent in links for parent in path.parents):
                raise ValueError("source member has a symlink ancestor")
            if item.issym():
                target = PurePosixPath(item.linkname)
                if target.is_absolute() or "\\" in item.linkname or \
                        any(ord(char) < 32 for char in item.linkname) or \
                        not within((tree / str(path.parent) / item.linkname).resolve(), tree.resolve()):
                    raise ValueError("source symlink escapes export")
            elif not item.isfile():
                raise ValueError("unsupported source archive entry")
        # No archive attributes, hard links, special files or symlink ancestors.
        tar.extractall(tree, members=members)


def export(root, destination, expected):
    root = root.resolve()
    destination = destination.resolve()
    if within(destination, root):
        raise ValueError("source export must be outside the checkout")
    if has_git_tree(destination.parent):
        raise ValueError("source export must be outside any Git working tree")
    source = clean_identity(root, expected)
    entries = committed_files(root, expected)
    commit_object = base64.b64encode(git(root, "cat-file", "commit", expected)).decode("ascii")
    require_committed_tree(entries, source, commit_object)
    destination.mkdir(parents=True, exist_ok=False)
    archive = destination / "source.tar"
    write_archive(root, archive, entries, source["epoch"])
    require_archive_tree(archive, entries)
    tree = destination / "source"
    tree.mkdir()
    extract_archive(archive, tree)
    if source != clean_identity(root, expected):
        raise ValueError("source checkout changed during export")
    if has_git_tree(tree):
        raise ValueError("export unexpectedly resolves to a Git working tree")
    receipt = {"schema": "nyxid.build-source-export.v1", "source": source,
               "archive": {"name": "source.tar", "sha256": digest(archive), "size": archive.stat().st_size},
               "commit_object_base64": commit_object, "committed_files": entries,
               "files": require_exported_files(tree, entries)}
    (destination / "source-export.json").write_text(json.dumps(receipt, indent=2) + "\n")
    return receipt


def unique(pairs):
    result = {}
    for name, value in pairs:
        if name in result:
            raise ValueError("duplicate export receipt key")
        result[name] = value
    return result


def verify(destination, expected, generated=()):
    with (destination / "source-export.json").open() as stream:
        receipt = json.load(stream, object_pairs_hook=unique)
    if receipt.get("schema") != "nyxid.build-source-export.v1" or \
            receipt.get("source", {}).get("commit") != expected:
        raise ValueError("source export receipt differs from expected identity")
    require_committed_tree(receipt["committed_files"], receipt["source"], receipt["commit_object_base64"])
    archive = destination / "source.tar"
    if receipt["archive"] != {"name": "source.tar", "sha256": digest(archive), "size": archive.stat().st_size}:
        raise ValueError("source archive differs from receipt")
    require_archive_tree(archive, receipt["committed_files"])
    if has_git_tree(destination / "source"):
        raise ValueError("source export resolves to a Git working tree")
    if require_exported_files(destination / "source", receipt["committed_files"], generated) != receipt["files"]:
        raise ValueError("exported source differs from receipt")
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("export", "verify"))
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--generated", action="append", default=[],
                        help="reviewed build-output directory, verification only")
    args = parser.parse_args()
    if args.operation == "export":
        if args.generated:
            parser.error("source export cannot exclude tracked inputs")
        result = export(args.root, args.destination, args.commit)
    else:
        result = verify(args.destination, args.commit, args.generated)
    print(json.dumps({"source": result["source"], "archive": result["archive"],
                      "files": len(result["files"])}))


if __name__ == "__main__":
    main()

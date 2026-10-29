#!/usr/bin/env python3
"""Export the Docker Machine policy as a fixed native amd64/arm64 OCI policy.

The hosted supervisor uses the fixed Docker-default capability baseline.
Docker-specific conditions must be resolved, not silently discarded by a CRI reader.
Only --install-root writes outside this artifact directory, and requires root
on a supported Linux node. It never changes an existing profile's contents.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import stat
import tempfile

REPOSITORY = Path(__file__).resolve().parents[3]
SOURCE = REPOSITORY / "cli/resources/machine-container/seccomp.json"
CAPABILITIES = {"CAP_" + name for name in json.loads(
    (REPOSITORY / "machine/resources/docker-supervisor-capabilities.json").read_text())}
ARCHES = {"amd64", "arm64"}
# These reviewed architecture-only names cannot grant a common syscall on the
# other native ABI. New architecture-dependent common rules need review: OCI
# has no per-rule Docker `arches` selector to express them in this shared file.
ARCH_ONLY_NAMES = {
    "amd64": {"arch_prctl", "modify_ldt"},
    "arm64": {"arm_fadvise64_64", "arm_sync_file_range", "sync_file_range2",
              "breakpoint", "cacheflush", "set_tls"},
}
MIN_KERNEL = (4, 8, 0)
REFERENCE = re.compile(r"nyxid/remote-machine-v1-[0-9a-f]{64}\.json\Z")


def kernel_tuple(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]+\.[0-9]+(?:\.[0-9]+)?", value):
        raise ValueError("invalid kernel selector")
    parts = tuple(map(int, value.split(".")))
    return parts + (0,) * (3 - len(parts))


def condition(selector, include, architecture):
    if not isinstance(selector, dict) or set(selector) - {"arches", "caps", "minKernel"}:
        raise ValueError("unsupported Docker seccomp selector")
    results = []
    for key in ["arches", "caps"]:
        if key in selector:
            if not isinstance(selector[key], list) or not selector[key] or not all(isinstance(v, str) for v in selector[key]):
                raise ValueError("invalid Docker seccomp selector")
            if key == "arches":
                results.append(architecture in selector[key])
            else:
                # Docker includes require every listed cap; excludes match any.
                checks = [cap in CAPABILITIES for cap in selector[key]]
                results.append(all(checks) if include else any(checks))
    if "minKernel" in selector:
        minimum = kernel_tuple(selector["minKernel"])
        if minimum > MIN_KERNEL:
            raise ValueError("new kernel condition needs an explicit profile review")
        results.append(True)
    return all(results) if include else any(results)


def export_policy(source):
    if set(source) != {"defaultAction", "defaultErrnoRet", "archMap", "syscalls"}:
        raise ValueError("source policy shape changed; review the exporter")
    if source["defaultAction"] != "SCMP_ACT_ERRNO" or source["defaultErrnoRet"] != 1:
        raise ValueError("the default deny policy must be preserved")
    rules = []
    for original in source["syscalls"]:
        if set(original) - {"names", "action", "args", "errnoRet", "comment", "includes", "excludes"}:
            raise ValueError("unsupported source rule field")
        selected = set()
        for architecture in ARCHES:
            # Validate both selectors even if the first one excludes this rule.
            include = condition(original.get("includes", {}), True, architecture)
            exclude = condition(original.get("excludes", {}), False, architecture)
            if include and not exclude:
                selected.add(architecture)
        if selected and selected != ARCHES:
            if not all(set(original["names"]) <= ARCH_ONLY_NAMES[arch] for arch in selected):
                raise ValueError("architecture-dependent common syscalls need an explicit profile review")
        if selected:
            rules.append({key: value for key, value in original.items()
                          if key in {"names", "action", "args", "errnoRet"}})
    return {"defaultAction": "SCMP_ACT_ERRNO", "defaultErrnoRet": 1,
            # No 32-bit compatibility ABIs. Names specific to one native
            # architecture have no syscall mapping on the other architecture.
            "architectures": ["SCMP_ARCH_X86_64", "SCMP_ARCH_AARCH64"], "syscalls": rules}


def artifact(policy):
    content = (json.dumps(policy, indent=2, sort_keys=True) + "\n").encode()
    digest = hashlib.sha256(content).hexdigest()
    return "nyxid/remote-machine-v1-" + digest + ".json", content


def install_artifact(root, reference, content):
    root = Path(root)
    expected = "nyxid/remote-machine-v1-" + hashlib.sha256(content).hexdigest() + ".json"
    if not REFERENCE.fullmatch(reference) or reference != expected:
        raise ValueError("profile reference does not bind its exact contents")
    if not root.is_absolute() or not root.is_dir() or root.resolve() != root:
        raise ValueError("kubelet seccomp root must be an existing absolute nonsymlink directory")
    for parent in [root, root / "nyxid"]:
        if parent == root / "nyxid" and not parent.exists():
            parent.mkdir(mode=0o755)
        metadata = parent.lstat()
        if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o022:
            raise ValueError("seccomp directories must be owned by the installer and not group/world writable")
    target = root / reference

    def validate_existing():
        metadata = target.lstat()
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o022:
            raise ValueError("existing seccomp profile has unsafe ownership or permissions")
        if target.read_bytes() != content:
            raise ValueError("existing immutable seccomp profile has different contents")

    if target.exists() or target.is_symlink():
        validate_existing()
        return target
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=target.parent, prefix=".remote-machine-", delete=False) as handle:
            temporary = Path(handle.name)
            os.fchmod(handle.fileno(), 0o644)
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        # Publish only complete contents, without replacing another installer's
        # immutable file. Both paths reside on the same kubelet filesystem.
        try:
            os.link(temporary, target)
        except FileExistsError:
            validate_existing()
        descriptor = os.open(target.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    finally:
        if temporary is not None:
            temporary.unlink()
    return target


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify checked-in artifacts without writing")
    parser.add_argument("--install-root", type=Path, help="install on this Linux node's kubelet seccomp root")
    args = parser.parse_args()
    reference, content = artifact(export_policy(json.loads(SOURCE.read_text())))
    manifest = {"version": 1, "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
                "profile_sha256": hashlib.sha256(content).hexdigest(), "localhost_profile": reference,
                "architectures": sorted(ARCHES), "minimum_kernel": "4.8",
                "capabilities": sorted(CAPABILITIES)}
    manifest_bytes = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
    directory = Path(__file__).resolve().parent
    artifacts = {directory / Path(reference).name: content, directory / "manifest.json": manifest_bytes}
    if args.check or args.install_root:
        for path, expected in artifacts.items():
            if not path.is_file() or path.read_bytes() != expected:
                raise ValueError("stale or missing generated seccomp artifact: " + str(path))
    else:
        for path, expected in artifacts.items():
            path.write_bytes(expected)
    if args.install_root:
        if platform.system() != "Linux" or platform.machine() not in {"x86_64", "aarch64"} or os.geteuid() != 0:
            raise ValueError("installation requires root on native Linux amd64 or arm64")
        release = re.match(r"[0-9]+\.[0-9]+(?:\.[0-9]+)?", platform.release())
        if not release or kernel_tuple(release.group()) < MIN_KERNEL:
            raise ValueError("the profile requires Linux >= 4.8; separated has stricter live requirements")
        install_artifact(args.install_root, reference, content)
    print(reference)


if __name__ == "__main__":
    main()

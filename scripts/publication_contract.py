"""Expected publication coverage, independent of downloaded artifact discovery.

This module validates coverage and digest bindings. Cryptographic verification
and scans are separate required producer jobs, whose outputs the final gate
must consume. A coverage receipt alone is not signing/scan evidence.
"""
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
POLICY = ROOT / "security/publication-policy.json"
DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON object key")
        result[key] = value
    return result


def load_json(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 16 * 1024 * 1024:
        raise ValueError("expected bounded regular JSON input")
    return json.loads(path.read_text(), object_pairs_hook=unique_object)


def source_identity(commit, epoch):
    if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}", commit) or type(epoch) is not int or epoch <= 0:
        raise ValueError("full source commit and positive integer commit epoch required")
    return {"commit": commit, "epoch": epoch}


def dist_contract(dist, metadata, commit, epoch, policy=None):
    policy = policy or load_json(POLICY)
    cli = policy["cli"]
    targets = cli["targets"]
    configured = metadata["metadata"]["dist"]
    if len(targets) != len(set(targets)) or sorted(configured["targets"]) != sorted(targets) or configured.get("cargo-auditable") is not True:
        raise ValueError("Cargo dist target/auditable policy mismatch")
    packages = [item for item in metadata["packages"] if item["name"] == cli["package"]]
    releases = dist.get("releases", [])
    if len(packages) != 1 or len(releases) != 1 or releases[0]["app_name"] != cli["package"] or releases[0]["app_version"] != packages[0]["version"]:
        raise ValueError("dist package/version differs from validated Cargo plan")
    version = packages[0]["version"]
    if dist.get("announcement_tag") != "v" + version:
        raise ValueError("dist tag differs from package version")
    matrix_targets = [target for item in dist["ci"]["github"]["artifacts_matrix"]["include"] for target in item["targets"]]
    if len(matrix_targets) != len(set(matrix_targets)) or sorted(matrix_targets) != sorted(targets):
        raise ValueError("dist build matrix omits/duplicates/adds planned targets")
    expected = dict(cli["global_artifacts"])
    archive_targets = {}
    for target in targets:
        archive = cli["package"] + "-" + target + ".tar.gz"
        expected[archive] = "executable-zip"
        expected[archive + ".sha256"] = "checksum"
        archive_targets[archive] = target
    artifacts = dist["artifacts"]
    release_names = releases[0]["artifacts"]
    if len(release_names) != len(set(release_names)) or set(artifacts) != set(expected) or set(release_names) != set(expected):
        raise ValueError("dist plan has missing/duplicate/extra publication artifacts")
    subjects = []
    for name, kind in sorted(expected.items()):
        artifact = artifacts[name]
        if artifact.get("name") != name or artifact.get("kind") != kind:
            raise ValueError("dist artifact identity differs from expected plan")
        subject = {"id": "file:" + name, "kind": "file", "name": name, "artifact_kind": kind}
        if name in archive_targets:
            target = archive_targets[name]
            assets = artifact.get("assets", [])
            paths = [item.get("path") for item in assets]
            executables = [item for item in assets if item.get("kind") == "executable"]
            if artifact.get("target_triples") != [target] or len(executables) != 1 or executables[0].get("path") != cli["binary"] or \
                    any(not isinstance(path, str) or not re.fullmatch(r"[A-Za-z0-9_.-]+", path) for path in paths) or len(paths) != len(set(paths)):
                raise ValueError("archive target/member coverage differs from expected plan")
            subject.update(target=target, assets=assets, binary=cli["binary"], package=cli["package"])
        subjects.append(subject)
    for name, kind in sorted(cli["supplemental_artifacts"].items()):
        subjects.append({"id": "file:" + name, "kind": "file", "name": name, "artifact_kind": kind})
    return {"schema": "nyxid.publication-plan.v1", "producer": "cli", "source": source_identity(commit, epoch),
            "version": version, "tag": "v" + version, "subjects": subjects}


def image_contract(commit, epoch, matrix, owner="chronoaiproject", policy=None):
    policy = policy or load_json(POLICY)
    images = policy["images"]
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", owner):
        raise ValueError("invalid image owner")
    if sorted(matrix.get("component", [])) != sorted(images["components"]) or sorted(matrix.get("platform", [])) != sorted(images["platforms"]):
        raise ValueError("image matrix differs from complete component/platform policy")
    definitions = [item for item in matrix.get("include", []) if "component" in item]
    if len(definitions) != len(images["components"]) or {item["component"]: item.get("file") for item in definitions} != images["components"]:
        raise ValueError("image matrix Dockerfile ownership differs from policy")
    subjects = []
    for component, dockerfile in sorted(images["components"].items()):
        repository = "ghcr.io/" + owner + "/nyxid/" + component
        for platform in images["platforms"]:
            subjects.append({"id": "image:" + component + ":" + platform, "kind": "image-child", "name": repository,
                             "component": component, "platform": platform, "dockerfile": dockerfile})
        subjects.append({"id": "index:" + component, "kind": "image-index", "name": repository, "component": component,
                         "platforms": images["platforms"]})
    return {"schema": "nyxid.publication-plan.v1", "producer": "images", "source": source_identity(commit, epoch), "subjects": subjects}


def require_success(results, expected_jobs):
    if set(results) != set(expected_jobs) or any(value != "success" for value in results.values()):
        raise ValueError("required publication job failed, skipped, cancelled, absent or unexpected")


def require_build_identity(receipt, package, version, target, source, local_investigation=False):
    """Compare actual retained-section identity with the independently planned subject.

    The extractor binds this identity to final binary bytes. Target ABI still
    requires actual executable/loader inspection; a target string is insufficient.
    """
    identity = receipt.get("build_identity")
    expected = {"schema": "nyxid.build-identity.v1", "package": package,
                "package_version": version, "target": target, "commit": source["commit"],
                "source_date_epoch": source["epoch"]}
    allowed = {"exported"} | ({"git-unverified", "git-dirty"} if local_investigation else set())
    if not isinstance(identity, dict) or set(identity) != set(expected) | {"source_state"} or \
            any(identity.get(key) != value for key, value in expected.items()) or \
            identity.get("source_state") not in allowed:
        raise ValueError("actual retained build identity differs from planned source/package/version/target or release state")
    return identity


def validate_coverage(plan, records):
    validate_plan(plan)
    expected = {item["id"]: item for item in plan["subjects"]}
    identifiers = [item["id"] for item in records]
    if len(expected) != len(plan["subjects"]) or len(identifiers) != len(set(identifiers)) or set(identifiers) != set(expected):
        raise ValueError("missing/duplicate/extra planned publication subject")
    actual = {}
    for item in records:
        contract = expected[item["id"]]
        if item.get("source") != plan["source"] or any(item.get(key) != value for key, value in contract.items() if key in ("kind", "name", "component", "platform", "target")):
            raise ValueError("stale source or mismatched planned subject identity")
        if not isinstance(item.get("digest"), str) or not DIGEST.fullmatch(item["digest"]):
            raise ValueError("missing planned subject digest")
        actual[item["id"]] = item
    for item in records:
        if item["kind"] != "image-index":
            continue
        manifests = item.get("manifests", [])
        platforms = [child.get("platform") for child in manifests]
        contract = expected[item["id"]]
        if len(platforms) != len(set(platforms)) or sorted(platforms) != sorted(contract["platforms"]):
            raise ValueError("index has missing/duplicate/extra platform subjects")
        for child in manifests:
            record = actual["image:" + contract["component"] + ":" + child["platform"]]
            if child.get("digest") != record["digest"]:
                raise ValueError("index child differs from validated platform subject")
    return actual


def validate_plan(plan):
    if plan.get("schema") != "nyxid.publication-plan.v1" or plan.get("producer") not in ("cli", "images"):
        raise ValueError("invalid expected publication plan")
    source_identity(plan["source"]["commit"], plan["source"]["epoch"])
    policy = load_json(POLICY)
    ids = [item["id"] for item in plan["subjects"]]
    if len(ids) != len(set(ids)):
        raise ValueError("duplicate expected publication subject")
    if plan["producer"] == "cli":
        cli = policy["cli"]
        if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", plan.get("version", "")) or plan.get("tag") != "v" + plan["version"]:
            raise ValueError("invalid expected CLI version/tag")
        expected = {**cli["global_artifacts"], **cli["supplemental_artifacts"]}
        for target in cli["targets"]:
            name = cli["package"] + "-" + target + ".tar.gz"
            expected[name] = "executable-zip"
            expected[name + ".sha256"] = "checksum"
        if set(ids) != {"file:" + name for name in expected}:
            raise ValueError("expected CLI plan omits/adds publication subjects")
        for item in plan["subjects"]:
            if item.get("name") not in expected or item.get("id") != "file:" + item["name"] or item.get("kind") != "file" or item.get("artifact_kind") != expected[item["name"]]:
                raise ValueError("invalid expected CLI subject identity")
            if item["artifact_kind"] == "executable-zip":
                if item.get("target") not in cli["targets"] or item["name"] != cli["package"] + "-" + item["target"] + ".tar.gz" or item.get("binary") != cli["binary"] or item.get("package") != cli["package"]:
                    raise ValueError("invalid expected archive target/binary")
                assets = [asset.get("path") for asset in item.get("assets", [])]
                if not assets or len(assets) != len(set(assets)) or cli["binary"] not in assets or any(not isinstance(path, str) or not re.fullmatch(r"[A-Za-z0-9_.-]+", path) or path in (".", "..") for path in assets):
                    raise ValueError("invalid expected archive member set")
    else:
        images = policy["images"]
        expected = {"image:" + component + ":" + platform for component in images["components"] for platform in images["platforms"]} | {"index:" + component for component in images["components"]}
        if set(ids) != expected:
            raise ValueError("expected image plan omits/adds component/platform subjects")
        for item in plan["subjects"]:
            component = item.get("component")
            if component not in images["components"] or not re.fullmatch(r"ghcr\.io/[a-z0-9][a-z0-9-]*/nyxid/" + re.escape(component), item.get("name", "")):
                raise ValueError("invalid expected image identity")
            if item["id"].startswith("index:"):
                if item["id"] != "index:" + component or item.get("kind") != "image-index" or item.get("platforms") != images["platforms"]:
                    raise ValueError("invalid expected image index platforms")
            elif item.get("kind") != "image-child" or item.get("platform") not in images["platforms"] or item["id"] != "image:" + component + ":" + item["platform"] or item.get("dockerfile") != images["components"][component]:
                raise ValueError("invalid expected image child")


def evidence_requirements(subject):
    required = {"provenance", "scan"}
    if subject["kind"] in ("image-child", "image-index") or subject.get("artifact_kind") in ("executable-zip", "credential-manifest"):
        required.add("sbom")
    if subject.get("artifact_kind") == "credential-manifest":
        required.update(("manifest-signature", "credential-composition"))
    if subject["kind"] == "image-child":
        required.add("distro-packages")
        if subject["component"] in ("frontend", "backend"):
            required.update(("credential-parity", "shipped-composition"))
        if subject["component"] in ("node-agent", "nyxid-node-machine"):
            required.add("embedded-wizard")
    return required


def validate_evidence_set(plan, records, directory):
    """Bind the complete evidence file closure; does not verify signatures.

    The producer runs actual cryptographic/schema/scan checks before invoking
    this coverage check. Its signed publication manifest binds these receipts.
    """
    actual = validate_coverage(plan, records)
    for subject in plan["subjects"]:
        record = actual[subject["id"]]
        evidence = record.get("evidence", {})
        if set(evidence) != evidence_requirements(subject):
            raise ValueError("missing/extra planned SBOM/signature/scan/composition evidence")
        paths = set()
        for item in evidence.values():
            name = item.get("path")
            if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*", name) or any(part in (".", "..") for part in name.split("/")) or name in paths:
                raise ValueError("invalid/duplicate staged evidence path")
            paths.add(name)
            path = Path(directory)
            for part in name.split("/"):
                path = path / part
                if path.is_symlink():
                    raise ValueError("symlink in staged evidence path")
            if file_digest(path, 64 * 1024 * 1024) != item.get("digest"):
                raise ValueError("stale/changed staged evidence bytes")
    return actual


def file_digest(path, maximum=1024 * 1024 * 1024):
    path = Path(path)
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= maximum:
        raise ValueError("expected bounded regular staged artifact")
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(65536), b""):
            digest.update(block)
    return "sha256:" + digest.hexdigest()

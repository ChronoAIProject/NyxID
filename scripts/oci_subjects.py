"""Validate OCI runnable-platform coverage and explicit BuildKit auxiliaries.

This validates digest-bound OCI JSON, not GitHub OIDC provenance. BuildKit's
unsigned embedded statement is recorded as auxiliary content and never used
as the official producer signature.
"""
import base64
import hashlib
import json
from pathlib import Path

from publication_contract import DIGEST, load_json, unique_object

OCI_INDEX = "application/vnd.oci.image.index.v1+json"
OCI_MANIFEST = "application/vnd.oci.image.manifest.v1+json"
DOCKER_INDEX = "application/vnd.docker.distribution.manifest.list.v2+json"
DOCKER_MANIFEST = "application/vnd.docker.distribution.manifest.v2+json"
CONFIGS = {"application/vnd.oci.image.config.v1+json", "application/vnd.docker.container.image.v1+json"}
LAYERS = {"application/vnd.oci.image.layer.v1.tar", "application/vnd.oci.image.layer.v1.tar+gzip", "application/vnd.oci.image.layer.v1.tar+zstd",
          "application/vnd.docker.image.rootfs.diff.tar.gzip"}
PREDICATES = {"https://slsa.dev/provenance/v0.2", "https://slsa.dev/provenance/v1", "https://spdx.dev/Document"}
MAX_JSON = 16 * 1024 * 1024


def declared_platform_matches(declared, config):
    if not isinstance(declared, dict) or set(declared) - {"os", "architecture", "variant"}:
        return False
    if any(declared.get(key) != config.get(key) for key in ("os", "architecture")):
        return False
    # OCI producers may spell the supported arm64 ABI as arm64/v8. Other
    # variants are outside the publication plan and cannot be discarded.
    variants = (None, "v8") if config.get("architecture") == "arm64" else (None,)
    return declared.get("variant") in variants and config.get("variant") in variants


def check_descriptor(descriptor):
    if not isinstance(descriptor.get("digest"), str) or not DIGEST.fullmatch(descriptor["digest"]) or type(descriptor.get("size")) is not int or descriptor["size"] <= 0 or descriptor.get("urls") or "data" in descriptor:
        raise ValueError("invalid/remote OCI content descriptor")


def directory_reader(directory):
    directory = Path(directory)
    def read(digest):
        if not DIGEST.fullmatch(digest):
            raise ValueError("invalid OCI blob digest")
        path = directory / "blobs" / "sha256" / digest.split(":")[1]
        for parent in (path, path.parent, path.parent.parent):
            if parent.is_symlink():
                raise ValueError("symlink in OCI metadata path")
        if not path.is_file() or path.stat().st_size > MAX_JSON:
            raise ValueError("missing/oversized OCI JSON blob")
        return path.read_bytes()
    return read


def capture_reader(capture):
    """Read a metadata-only capture of actual docker-save OCI JSON bytes."""
    records = capture["records"]
    def read(digest):
        record = records["blobs/sha256/" + digest.split(":")[1]]
        data = base64.b64decode(record["base64"], validate=True)
        if hashlib.sha256(data).hexdigest() != record["sha256"]:
            raise ValueError("changed captured OCI bytes")
        return data
    return read


def inspect(root, read_blob, expected_platforms, expected_commit=None):
    if not expected_platforms or len(expected_platforms) != len(set(expected_platforms)) or not set(expected_platforms) <= {"linux/amd64", "linux/arm64"}:
        raise ValueError("invalid expected runnable platform set")
    runnable, auxiliaries, envelopes, visited = {}, [], [], set()
    pending_attestations = []

    def value(descriptor):
        check_descriptor(descriptor)
        data = read_blob(descriptor["digest"])
        if len(data) != descriptor["size"] or len(data) > MAX_JSON or "sha256:" + hashlib.sha256(data).hexdigest() != descriptor["digest"]:
            raise ValueError("OCI metadata bytes differ from content descriptor")
        return json.loads(data, object_pairs_hook=unique_object)

    def manifest(descriptor, inherited_platform=None, depth=0):
        if depth > 2 or descriptor["digest"] in visited:
            raise ValueError("nested/duplicate OCI subject descriptor")
        visited.add(descriptor["digest"])
        document = value(descriptor)
        media = descriptor["mediaType"]
        if document.get("schemaVersion") != 2 or document.get("mediaType", media) != media:
            raise ValueError("OCI schema/media type mismatch")
        if media in (OCI_INDEX, DOCKER_INDEX):
            children = document.get("manifests", [])
            if not 1 <= len(children) <= 16:
                raise ValueError("OCI index descriptor count exceeds policy")
            envelopes.append({"digest": descriptor["digest"], "manifests": children})
            for child in children:
                declared = child.get("platform")
                if child.get("annotations", {}).get("vnd.docker.reference.type") == "attestation-manifest":
                    pending_attestations.append(child)
                else:
                    manifest(child, declared or inherited_platform, depth + 1)
            return
        if media not in (OCI_MANIFEST, DOCKER_MANIFEST) or document.get("artifactType") or document.get("subject"):
            raise ValueError("unsupported OCI runnable manifest")
        config = document["config"]
        if config.get("mediaType") not in CONFIGS:
            raise ValueError("unsupported OCI runnable config")
        config_value = value(config)
        platform = config_value.get("os", "") + "/" + config_value.get("architecture", "")
        declared = descriptor.get("platform") or inherited_platform
        if platform not in expected_platforms or platform in runnable or (declared is not None and not declared_platform_matches(declared, config_value)):
            raise ValueError("missing/duplicate/extra/mismatched OCI runnable platform")
        if expected_commit is not None and config_value.get("config", {}).get("Labels", {}).get("org.opencontainers.image.revision") != expected_commit:
            raise ValueError("OCI image config source revision differs from expected commit")
        layers = document.get("layers", [])
        if not 1 <= len(layers) <= 256:
            raise ValueError("invalid OCI runnable layer set")
        for layer in layers:
            check_descriptor(layer)
            if layer.get("mediaType") not in LAYERS:
                raise ValueError("unsupported OCI runnable layer")
        runnable[platform] = {"platform": platform, "digest": descriptor["digest"], "config": config["digest"]}

    manifest(root)
    if set(runnable) != set(expected_platforms):
        raise ValueError("missing planned OCI runnable platform")
    runnable_digests = {item["digest"] for item in runnable.values()}
    if len(pending_attestations) > len(runnable):
        raise ValueError("extra OCI auxiliary manifests")
    referenced = set()
    for descriptor in pending_attestations:
        annotations = descriptor.get("annotations", {})
        reference = annotations.get("vnd.docker.reference.digest")
        if descriptor.get("platform") != {"architecture": "unknown", "os": "unknown"} or set(annotations) != {"vnd.docker.reference.digest", "vnd.docker.reference.type"} or reference not in runnable_digests or reference in referenced or descriptor.get("mediaType") != OCI_MANIFEST or descriptor["digest"] in visited:
            raise ValueError("unbound/duplicate/unsupported OCI attestation descriptor")
        referenced.add(reference)
        visited.add(descriptor["digest"])
        document = value(descriptor)
        if document.get("schemaVersion") != 2 or document.get("mediaType") != OCI_MANIFEST or document.get("subject") or document.get("artifactType"):
            raise ValueError("unsupported OCI auxiliary manifest")
        config = document["config"]
        if config.get("mediaType") not in CONFIGS:
            raise ValueError("unsupported OCI auxiliary config")
        configuration = value(config)
        if configuration.get("architecture") != "unknown" or configuration.get("os") != "unknown":
            raise ValueError("OCI auxiliary config has runnable platform")
        layers = document.get("layers", [])
        if not 1 <= len(layers) <= len(PREDICATES):
            raise ValueError("invalid OCI auxiliary layer count")
        predicates = []
        for layer in layers:
            predicate = layer.get("annotations", {}).get("in-toto.io/predicate-type")
            if layer.get("mediaType") != "application/vnd.in-toto+json" or predicate not in PREDICATES or predicate in [item["predicate"] for item in predicates]:
                raise ValueError("unsupported/duplicate OCI auxiliary predicate")
            statement = value(layer)
            subjects = statement.get("subject", [])
            if statement.get("_type") not in ("https://in-toto.io/Statement/v0.1", "https://in-toto.io/Statement/v1") or statement.get("predicateType") != predicate or len(subjects) != 1 or subjects[0].get("digest") != {"sha256": reference.split(":")[1]}:
                raise ValueError("OCI auxiliary statement differs from referenced runnable subject")
            predicates.append({"digest": layer["digest"], "predicate": predicate})
        if configuration.get("rootfs") != {"type": "layers", "diff_ids": [layer["digest"] for layer in layers]}:
            raise ValueError("OCI auxiliary config does not bind its statement layers")
        auxiliaries.append({"digest": descriptor["digest"], "reference": reference, "predicates": predicates})
    return {"schema": "nyxid.oci-subjects.v1", "subject": {"digest": root["digest"], "mediaType": root["mediaType"]},
            "runnable": [runnable[key] for key in sorted(runnable)], "auxiliaries": auxiliaries, "envelopes": envelopes}

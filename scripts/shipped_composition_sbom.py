"""CycloneDX inclusion records from a previously verified final-byte seal.

The caller must first run the Node source/seal verifier, then establish that
these final bytes occur in its actual archive/image/binary subject.
"""
import base64
import json


def components(seal, prefix):
    if seal.get("schema") != "nyxid.shipped-composition.v1":
        raise ValueError("unsupported composition seal")
    membership = {}
    for graph in seal["graphs"]:
        modules = [module for chunk in graph["chunks"] for module in chunk["modules"]] + graph["styles"]["sources"]
        for module in modules:
            identity = module.get("package", module.get("generated_by"))
            if identity:
                membership.setdefault(identity["purl"], set()).add(module["id"])
    identities = {item["purl"]: item for item in seal["components"]}
    if set(membership) != set(identities):
        raise ValueError("package inclusion lacks retained module/style membership")
    result = []
    refs = []
    for purl, item in sorted(identities.items()):
        if purl != "pkg:npm/" + item["name"].replace("@", "%40", 1) + "@" + item["version"]:
            raise ValueError("noncanonical included package identity")
        algorithm, integrity = item["integrity"].split("-", 1)
        if algorithm not in ("sha512", "sha256"):
            raise ValueError("unsupported included archive integrity")
        digest = base64.b64decode(integrity, validate=True)
        if len(digest) != {"sha512": 64, "sha256": 32}[algorithm]:
            raise ValueError("invalid included archive integrity length")
        ref = prefix + purl
        refs.append(ref)
        result.append({"type": "library", "bom-ref": ref, "name": item["name"], "version": item["version"], "purl": purl,
            "hashes": [{"alg": {"sha512": "SHA-512", "sha256": "SHA-256"}[algorithm], "content": digest.hex()}],
            "evidence": {"occurrences": [{"location": name} for name in sorted(membership[purl])]},
            "properties": [{"name": "nyxid:inventory-source", "value": "retained JS/CSS or generated runtime membership in verified final-byte seal"}]})
    for artifact in seal["artifacts"]:
        ref = prefix + "file:" + artifact["path"]
        refs.append(ref)
        component = {"type": "file", "bom-ref": ref, "name": artifact["path"],
            "hashes": [{"alg": "SHA-256", "content": artifact["sha256"]}],
            "properties": [{"name": "nyxid:size-bytes", "value": str(artifact["size"])}]}
        if artifact.get("source"):
            component["properties"].append({"name": "nyxid:source", "value": artifact["source"]})
        if artifact.get("provenance"):
            component["properties"].append({"name": "nyxid:static-provenance", "value": json.dumps(artifact["provenance"], sort_keys=True)})
            component["licenses"] = [{"license": {"id": artifact["provenance"]["license"]}}]
        html = next((item for item in seal["html"] if item["path"] == artifact["path"]), None)
        if html:
            component["properties"].append({"name": "nyxid:html-attribution", "value": json.dumps(html, sort_keys=True)})
        result.append(component)
    return result, refs

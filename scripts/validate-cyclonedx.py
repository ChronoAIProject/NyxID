#!/usr/bin/env python3
"""Validate CycloneDX 1.6 offline against the pinned official JSON schemas.

Install security/python-sbom-requirements.txt with --require-hashes and
--only-binary=:all:. No schema is fetched while validating an artifact.
"""
import argparse
import hashlib
import json
from pathlib import Path

from jsonschema import Draft7Validator
from referencing import Registry, Resource

SCHEMAS = Path(__file__).resolve().parents[1] / "security/schemas/cyclonedx-1.6"


def validator(directory=SCHEMAS):
    provenance = json.loads((directory / "provenance.json").read_text())
    registry = Registry()
    schema = None
    for record in provenance["files"]:
        data = (directory / record["path"]).read_bytes()
        if hashlib.sha256(data).hexdigest() != record["sha256"]:
            raise ValueError("CycloneDX schema differs from pinned upstream bytes")
        if record["path"].endswith(".schema.json"):
            value = json.loads(data)
            Draft7Validator.check_schema(value)
            registry = registry.with_resource(value["$id"], Resource.from_contents(value))
            if record["path"] == "bom-1.6.schema.json":
                schema = value
    if schema is None:
        raise ValueError("missing CycloneDX schema")
    return Draft7Validator(schema, registry=registry)


def validate(document):
    validator().validate(document)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("documents", type=Path, nargs="+")
    args = parser.parse_args()
    check = validator()
    for path in args.documents:
        check.validate(json.loads(path.read_text()))
        print(path.name + ": official CycloneDX 1.6 schema passed")

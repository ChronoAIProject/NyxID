#!/usr/bin/env python3
"""Fail when a reusable workflow's jobs request more than their caller grants.

GitHub validates nested job permissions only when the caller starts. A job in
ci.yml that asks for a scope the Publish Images `ci-gate` call does not grant
passes every pull request (ci.yml runs directly there) and then makes Publish
Images on main and tags fail with `startup_failure`. Check it on PRs instead.
"""
import sys
from pathlib import Path

from workflow_yaml import load_unique

LEVELS = {"none": 0, "read": 1, "write": 2}
WORKFLOWS = Path(".github/workflows")


def permissions(block, where):
    """Normalise a permissions block to {scope: level}; None means inherit."""
    if block is None:
        return None
    if isinstance(block, str):
        if block in ("read-all", "write-all"):
            sys.exit(f"{where}: use explicit scopes instead of {block}")
        sys.exit(f"{where}: unsupported permissions value {block!r}")
    result = {}
    for scope, level in block.items():
        if level not in LEVELS:
            sys.exit(f"{where}: unsupported level {level!r} for {scope}")
        result[scope] = LEVELS[level]
    return result


def validate_permission_scopes(workflow):
    """Every job must have an explicit scope, directly or at workflow level."""
    failures = []
    workflow_grant = permissions(workflow.get('permissions'), 'workflow')
    for name, job in (workflow.get('jobs') or {}).items():
        grant = permissions(job.get('permissions'), name)
        if grant is None and workflow_grant is None:
            failures.append(f'{name}: missing explicit permissions at job or workflow level')
    return failures


def main():
    failures = []
    callers = 0
    for caller_path in sorted(path for path in WORKFLOWS.iterdir() if path.suffix in (".yml", ".yaml")):
        caller = load_unique(caller_path.read_text())
        failures.extend(f'{caller_path}:{failure}' for failure in validate_permission_scopes(caller))
        workflow_grant = permissions(caller.get("permissions"), caller_path)
        for job_name, job in (caller.get("jobs") or {}).items():
            uses = job.get("uses", "")
            if not uses.startswith("./.github/workflows/"):
                continue
            callers += 1
            where = f"{caller_path}:{job_name}"
            grant = permissions(job.get("permissions"), where)
            if grant is None:
                grant = workflow_grant
            if grant is None:
                # Inheriting the repository default is not checkable here.
                continue
            called_path = Path(uses.removeprefix("./"))
            called = load_unique(called_path.read_text())
            requests = [(f"{called_path} (workflow)", called.get("permissions"))]
            requests += [
                (f"{called_path}:{name}", nested.get("permissions"))
                for name, nested in (called.get("jobs") or {}).items()
            ]
            for nested_where, block in requests:
                for scope, level in (permissions(block, nested_where) or {}).items():
                    if level > grant.get(scope, 0):
                        failures.append(
                            f"{nested_where} requests {scope}: "
                            f"{[k for k, v in LEVELS.items() if v == level][0]}, "
                            f"but {where} grants "
                            f"{[k for k, v in LEVELS.items() if v == grant.get(scope, 0)][0]}"
                        )
    for failure in failures:
        print(f"::error::{failure}")
    if failures:
        return 1
    print(f"Reusable workflow permissions fit their callers ({callers} calls checked).")
    return 0


if __name__ == "__main__":
    sys.exit(main())

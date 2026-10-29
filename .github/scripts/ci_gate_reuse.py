#!/usr/bin/env python3
"""Decide whether a publish CI gate can reuse an identical PR/main run.

The decision is deliberately conservative. GitHub metadata identifies the
candidate run and its jobs; git objects identify the tree tested by that run.
No artifact, job log, or PR-produced text participates in the decision.
"""

from __future__ import annotations

import argparse
import itertools
import json
import re
import subprocess
import sys
from collections import Counter
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Any, Callable, Iterable, Mapping


WORKFLOW_PATH = ".github/workflows/ci.yml"
PUBLISH_WORKFLOW_PATH = ".github/workflows/publish-images.yml"
CI_PIPELINE_NAME = "CI Pipeline"
CI_GATE_PREFIX = "CI Gate / "


class WorkflowSchemaError(ValueError):
    """A YAML construct whose GitHub Actions meaning cannot be proven."""

    def __init__(self, reason: str):
        self.reason = reason
        super().__init__(reason)


class WorkflowIdentityError(ValueError):
    """A workflow/job identity cannot be mapped without ambiguity."""

    def __init__(self, reason: str):
        self.reason = reason
        super().__init__(reason)


def strict_load_workflow(text: str) -> Mapping[str, Any]:
    """Load workflow YAML with only deterministic, GitHub-safe constructs.

    PyYAML's default YAML 1.1 resolver treats ``on`` as a boolean. The
    resolver below uses YAML 1.2 boolean words so the workflow key remains the
    string GitHub parses. Anchors, aliases, explicit tags, merge keys,
    duplicate keys, non-string keys, and non-core scalar types are rejected.
    """

    try:
        import yaml
        from yaml.events import AliasEvent
        from yaml.nodes import MappingNode, ScalarNode
    except ImportError as exc:  # pragma: no cover - setup failure is fail-open
        raise WorkflowSchemaError("yaml_dependency_missing") from exc

    core_tags = {
        "tag:yaml.org,2002:map",
        "tag:yaml.org,2002:seq",
        "tag:yaml.org,2002:str",
        "tag:yaml.org,2002:int",
        "tag:yaml.org,2002:float",
        "tag:yaml.org,2002:bool",
        "tag:yaml.org,2002:null",
        "tag:yaml.org,2002:merge",
    }
    bool_tag = "tag:yaml.org,2002:bool"

    class StrictLoader(yaml.SafeLoader):
        # Do not mutate SafeLoader's process-global resolver table.
        yaml_implicit_resolvers = {
            key: list(value) for key, value in yaml.SafeLoader.yaml_implicit_resolvers.items()
        }

        def compose_node(self, parent: Any, index: Any) -> Any:
            event = self.peek_event()
            if isinstance(event, AliasEvent):
                raise WorkflowSchemaError("yaml_alias")
            if getattr(event, "anchor", None):
                raise WorkflowSchemaError("yaml_anchor")
            # Explicit tags are not accepted even when they name a core type.
            if getattr(event, "tag", None) is not None:
                raise WorkflowSchemaError("yaml_tag")
            node = super().compose_node(parent, index)
            if node.tag not in core_tags:
                raise WorkflowSchemaError("yaml_non_core_scalar")
            return node

        def construct_mapping(self, node: Any, deep: bool = False) -> dict[str, Any]:
            if not isinstance(node, MappingNode):
                raise WorkflowSchemaError("yaml_mapping_expected")
            result: dict[str, Any] = {}
            seen: set[str] = set()
            for key_node, value_node in node.value:
                if isinstance(key_node, ScalarNode) and (
                    key_node.tag == "tag:yaml.org,2002:merge" or key_node.value == "<<"
                ):
                    raise WorkflowSchemaError("yaml_merge_key")
                if not isinstance(key_node, ScalarNode) or key_node.tag != "tag:yaml.org,2002:str":
                    raise WorkflowSchemaError("yaml_non_string_mapping_key")
                key = self.construct_object(key_node, deep=True)
                if not isinstance(key, str):
                    raise WorkflowSchemaError("yaml_non_string_mapping_key")
                if key == "<<":
                    raise WorkflowSchemaError("yaml_merge_key")
                if key in seen:
                    raise WorkflowSchemaError("yaml_duplicate_mapping_key")
                seen.add(key)
                result[key] = self.construct_object(value_node, deep=deep)
            return result

    # YAML 1.1's yes/no/on/off boolean resolver is a parser differential for
    # Actions. Retain only YAML 1.2 true/false resolution.
    for first, resolvers in list(StrictLoader.yaml_implicit_resolvers.items()):
        StrictLoader.yaml_implicit_resolvers[first] = [
            (tag, pattern) for tag, pattern in resolvers if tag != bool_tag
        ]
    StrictLoader.add_implicit_resolver(
        bool_tag,
        re.compile(r"^(?:true|True|TRUE|false|False|FALSE)$"),
        list("tTfF"),
    )

    try:
        value = yaml.load(text, Loader=StrictLoader)
    except WorkflowSchemaError:
        raise
    except yaml.YAMLError as exc:
        raise WorkflowSchemaError("yaml_syntax") from exc
    if not isinstance(value, dict):
        raise WorkflowSchemaError("yaml_root_mapping_expected")
    return value


@dataclass(frozen=True)
class Decision:
    reuse: bool
    reason: str
    source: str
    run_id: int | None = None
    run_url: str | None = None
    head_sha: str | None = None
    current_tree: str | None = None
    tested_tree: str | None = None
    candidates: tuple[str, ...] = ()
    jobs: tuple[str, ...] = ()
    run_jobs: tuple[str, ...] = ()

    def payload(self) -> dict[str, Any]:
        return {
            "reuse": self.reuse,
            "reason": self.reason,
            "source": self.source,
            "run_id": self.run_id,
            "run_url": self.run_url,
            "head_sha": self.head_sha,
            "current_tree": self.current_tree,
            "tested_tree": self.tested_tree,
            "candidates": list(self.candidates),
            "jobs": list(self.jobs),
            "run_jobs": list(self.run_jobs),
        }


def _fallback(reason: str, source: str = "none", **kwargs: Any) -> Decision:
    return Decision(False, reason, source, **kwargs)


def _parse_time(value: str | None) -> datetime:
    if not value:
        return datetime.min.replace(tzinfo=timezone.utc)
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def newest_completed_successful_run(
    runs: Iterable[Mapping[str, Any]], head_sha: str
) -> Mapping[str, Any] | None:
    """Return the newest attempt only when that attempt completed successfully."""

    matching = [run for run in runs if run.get("head_sha") == head_sha]
    if not matching:
        return None
    matching.sort(
        key=lambda run: (
            int(run.get("run_number") or 0),
            int(run.get("run_attempt") or 0),
            _parse_time(run.get("updated_at") or run.get("created_at")),
        ),
        reverse=True,
    )
    newest = matching[0]
    if newest.get("status") != "completed" or newest.get("conclusion") != "success":
        return None
    return newest


@dataclass(frozen=True)
class JobIdentity:
    job_id: str
    exact_names: tuple[str, ...]
    prefix: str | None
    reusable: bool
    supported: bool
    dynamic_prefix: str | None = None
    # Leaves a reusable child declares but disables for these static inputs.
    # GitHub still lists them; they are tolerated only as ``skipped``.
    skipped_names: tuple[str, ...] = ()


@dataclass(frozen=True)
class WorkflowIdentity:
    specs: tuple[JobIdentity, ...]
    reusable: Mapping[str, JobIdentity]
    candidates: tuple[str, ...]
    leaf_names: tuple[str, ...]
    complete: bool
    skipped_leaf_names: tuple[str, ...] = ()


_EXPRESSION = re.compile(r"\$\{\{.*?\}\}")
_MATRIX_EXPRESSION = re.compile(
    r"\$\{\{\s*matrix\.([A-Za-z_][A-Za-z0-9_]*)\s*\}\}"
)


def _has_event_sensitive_steps(job: Mapping[str, Any]) -> bool:
    """Keep jobs whose PR steps differ from a force-all main invocation."""

    details = {key: value for key, value in job.items()
               if key not in {"name", "if", "needs", "strategy", "uses"}}
    text = json.dumps(details, sort_keys=True)
    return any(marker in text for marker in ("github.event_name", "github.ref", "inputs.force-all"))


def _runs_under_force_all(job_id: str, job: Mapping[str, Any]) -> bool:
    """Exclude jobs whose own condition prevents a force-all invocation."""

    if job_id == "changes":
        return False
    condition = str(job.get("if") or "")
    if not condition:
        return True
    # A condition tied only to the pull_request event remains skipped on main;
    # it cannot be credited as a reusable force-all check.
    return "github.event_name" not in condition or "inputs.force-all" in condition


def _name_kind(name: str) -> tuple[str, tuple[str, ...]]:
    if "${{" not in name and "}}" not in name:
        return "static", ()
    expressions = list(_EXPRESSION.finditer(name))
    if (not expressions or "${{" in name[expressions[-1].end():]
            or "}}" in name[expressions[-1].end():]
            or "}}" in name[:expressions[0].start()]):
        return "dynamic", ()
    variables: list[str] = []
    cursor = 0
    for expression in expressions:
        if expression.start() < cursor:
            return "dynamic", ()
        match = _MATRIX_EXPRESSION.fullmatch(expression.group(0))
        if not match:
            return "dynamic", ()
        variables.append(match.group(1))
        cursor = expression.end()
    return "matrix", tuple(variables)


def _dynamic_prefix(name: str) -> str:
    marker = name.find("${{")
    return name if marker < 0 else name[:marker]


def _matrix_value(value: Any) -> str:
    if isinstance(value, bool) or not isinstance(value, (str, int)):
        raise WorkflowIdentityError("workflow_identity_matrix_value")
    return str(value)


def _expand_direct_names(job_id: str, job: Mapping[str, Any]) -> tuple[tuple[str, ...], bool, str | None]:
    if "name" not in job:
        name = job_id
    else:
        name = job.get("name")
        if not isinstance(name, str) or not name:
            raise WorkflowIdentityError("workflow_identity_job_name")
    kind, variables = _name_kind(name)
    strategy = job.get("strategy")
    if strategy is None:
        if kind == "static":
            return (name,), True, None
        return (), False, _dynamic_prefix(name)
    if not isinstance(strategy, Mapping):
        return (), False, _dynamic_prefix(name)
    matrix = strategy.get("matrix")
    if not isinstance(matrix, Mapping):
        return (), False, _dynamic_prefix(name)
    if "include" in matrix or "exclude" in matrix:
        return (), False, _dynamic_prefix(name)
    if not matrix or any(not isinstance(key, str) or not isinstance(value, list)
                         for key, value in matrix.items()):
        return (), False, _dynamic_prefix(name)
    if kind == "static":
        variables = ()
    elif kind != "matrix" or any(variable not in matrix for variable in variables):
        return (), False, _dynamic_prefix(name)
    values: list[list[str]] = []
    for key in matrix:
        converted = [_matrix_value(value) for value in matrix[key]]
        if not converted:
            return (), False, _dynamic_prefix(name)
        values.append(converted)
    keys = list(matrix)
    rows = [dict(zip(keys, row)) for row in itertools.product(*values)]
    names = tuple(
        name if kind == "static" else _MATRIX_EXPRESSION.sub(
            lambda match: rows[index][match.group(1)], name
        )
        for index, _ in enumerate(rows)
    )
    return names, True, None


def _resolve_local_workflow(
    current_path: str, uses: Any, workflows: Mapping[str, Mapping[str, Any]] | None
) -> tuple[str, Mapping[str, Any]]:
    if not isinstance(uses, str) or not uses.startswith("./"):
        raise WorkflowIdentityError("workflow_identity_external_reusable")
    if uses.startswith("./.github/"):
        child_path = uses[2:]
    else:
        child_path = (PurePosixPath(current_path).parent / uses[2:]).as_posix()
    if child_path.startswith("../") or workflows is None or child_path not in workflows:
        raise WorkflowIdentityError("workflow_identity_reusable_workflow_missing")
    return child_path, workflows[child_path]


def _workflow_call_inputs(
    workflow: Mapping[str, Any], provided: Any
) -> Mapping[str, Any] | None:
    """Resolve static workflow_call inputs for nested job conditions.

    Reusable workflows may contain jobs such as the ordinary-only billing
    smoke. Keep those leaves in the ordinary parent identity while excluding
    them from a coverage parent whose ``with.coverage`` is true. Unknown or
    expression-valued inputs remain unresolved and are handled conservatively.
    """

    trigger = workflow.get("on")
    call = trigger.get("workflow_call") if isinstance(trigger, Mapping) else None
    definitions = call.get("inputs") if isinstance(call, Mapping) else None
    values: dict[str, Any] = {}
    if isinstance(definitions, Mapping):
        for name, definition in definitions.items():
            if not isinstance(name, str) or not isinstance(definition, Mapping):
                return None
            default = definition.get("default")
            if default is None and definition.get("type") == "boolean":
                default = False
            values[name] = default
    if provided is None:
        return values
    if not isinstance(provided, Mapping):
        return None
    for name, value in provided.items():
        if not isinstance(name, str) or not isinstance(value, (bool, int, float, str)):
            return None
        values[name] = value
    return values


def _nested_job_enabled(job: Mapping[str, Any], inputs: Mapping[str, Any] | None) -> bool:
    """Evaluate the small input-only condition subset used by nested jobs."""

    condition = job.get("if")
    if not isinstance(condition, str) or inputs is None:
        return True
    condition = condition.strip()
    match = re.fullmatch(r"(!\s*)?inputs\.([A-Za-z_][A-Za-z0-9_-]*)", condition)
    if not match:
        return True
    value = inputs.get(match.group(2))
    if not isinstance(value, bool):
        return True
    return not value if match.group(1) else value


def _validate_identity_patterns(specs: Iterable[JobIdentity]) -> None:
    specs = tuple(specs)
    exact_owner: dict[str, str] = {}
    for spec in specs:
        if len(set(spec.exact_names)) != len(spec.exact_names):
            raise WorkflowIdentityError("workflow_identity_duplicate_display_name")
        for name in spec.exact_names:
            owner = exact_owner.setdefault(name, spec.job_id)
            if owner != spec.job_id:
                raise WorkflowIdentityError("workflow_identity_duplicate_display_name")

    prefixes = [(spec.job_id, spec.prefix) for spec in specs if spec.prefix]
    for index, (job_id, prefix) in enumerate(prefixes):
        assert prefix is not None
        for other_id, other in prefixes[index + 1:]:
            assert other is not None
            if prefix.startswith(other) or other.startswith(prefix):
                raise WorkflowIdentityError("workflow_identity_prefix_overlap")
        for name, owner in exact_owner.items():
            if owner != job_id and name.startswith(prefix):
                raise WorkflowIdentityError("workflow_identity_prefix_overlap")

    dynamic = [(spec.job_id, spec.dynamic_prefix) for spec in specs if spec.dynamic_prefix]
    for job_id, prefix in dynamic:
        assert prefix is not None
        for other_id, other in prefixes:
            assert other is not None
            if prefix.startswith(other) or other.startswith(prefix):
                raise WorkflowIdentityError("workflow_identity_dynamic_name_overlap")
        for name, owner in exact_owner.items():
            if owner != job_id and (name.startswith(prefix) or prefix.startswith(name)):
                raise WorkflowIdentityError("workflow_identity_dynamic_name_overlap")


def analyze_workflow(
    workflow: Mapping[str, Any],
    reusable_workflows: Mapping[str, Mapping[str, Any]] | None = None,
    *,
    current_path: str = WORKFLOW_PATH,
    nested: bool = False,
    input_values: Mapping[str, Any] | None = None,
    _stack: tuple[str, ...] = (),
) -> WorkflowIdentity:
    if current_path in _stack:
        raise WorkflowIdentityError("workflow_identity_reusable_cycle")
    stack = (*_stack, current_path)
    jobs = workflow.get("jobs")
    if not isinstance(jobs, Mapping) or any(not isinstance(key, str) for key in jobs):
        raise WorkflowIdentityError("workflow_identity_jobs_mapping")
    if nested:
        required = set(jobs)
    else:
        ci_pass = jobs.get("ci-pass")
        needs = ci_pass.get("needs") if isinstance(ci_pass, Mapping) else None
        if not isinstance(needs, list) or any(not isinstance(value, str) for value in needs):
            raise WorkflowIdentityError("workflow_identity_ci_pass_needs")
        required = set(needs)

    specs: list[JobIdentity] = []
    candidates: list[str] = []
    skipped_leaf_names: list[str] = []
    for job_id, raw_job in jobs.items():
        if not isinstance(raw_job, Mapping):
            raise WorkflowIdentityError("workflow_identity_job_mapping")
        if nested and not _nested_job_enabled(raw_job, input_values):
            # Only a plain static name can be matched to the API's skipped job.
            skipped_name = raw_job.get("name", job_id)
            if not isinstance(skipped_name, str) or not skipped_name or "${{" in skipped_name:
                raise WorkflowIdentityError("workflow_identity_skipped_job_name")
            skipped_leaf_names.append(skipped_name)
            continue
        if "if" in raw_job and not isinstance(raw_job.get("if"), str):
            raise WorkflowIdentityError("workflow_identity_if_expression")
        # A job in ci-pass.needs is eligible only when force-all would execute
        # it. This also supports older workflow files that predate reuse guards;
        # those callers ignore the input and run the eligible job normally.
        eligible = nested or (job_id in required and _runs_under_force_all(job_id, raw_job))
        event_sensitive = (not nested and not job_id.startswith("coverage-")
                           and _has_event_sensitive_steps(raw_job))
        if "uses" in raw_job:
            if "name" not in raw_job:
                parent_name = job_id
            else:
                parent_name = raw_job.get("name")
                if not isinstance(parent_name, str) or not parent_name:
                    raise WorkflowIdentityError("workflow_identity_job_name")
            kind, _ = _name_kind(parent_name)
            if kind != "static":
                spec = JobIdentity(job_id, (), None, False, False, _dynamic_prefix(parent_name))
            else:
                child_path, child_workflow = _resolve_local_workflow(
                    current_path, raw_job.get("uses"), reusable_workflows
                )
                child = analyze_workflow(
                    child_workflow, reusable_workflows, current_path=child_path,
                    nested=True,
                    input_values=_workflow_call_inputs(child_workflow, raw_job.get("with")),
                    _stack=stack
                )
                prefix = f"{parent_name} / "
                names = tuple(f"{prefix}{name}" for name in child.leaf_names)
                supported = child.complete and bool(names)
                spec = JobIdentity(job_id, names if supported else (), prefix,
                                   eligible and supported and not event_sensitive, supported,
                                   None if supported else prefix,
                                   tuple(f"{prefix}{name}" for name in child.skipped_leaf_names))
        else:
            names, supported, dynamic_prefix = _expand_direct_names(job_id, raw_job)
            spec = JobIdentity(job_id, names if supported else (), None,
                               eligible and supported and not event_sensitive, supported, dynamic_prefix)
        specs.append(spec)
        if eligible:
            candidates.append(job_id)

    _validate_identity_patterns(specs)
    reusable = {spec.job_id: spec for spec in specs if spec.reusable}
    leaf_names = tuple(name for spec in specs for name in spec.exact_names)
    complete = all(spec.supported for spec in specs)
    return WorkflowIdentity(tuple(specs), reusable, tuple(sorted(candidates)), leaf_names, complete,
                            tuple(skipped_leaf_names))


def expected_job_names(
    workflow: Mapping[str, Any],
    reusable_workflows: Mapping[str, Mapping[str, Any]] | None = None,
) -> dict[str, tuple[str, ...]]:
    """Return exact leaf names for eligible jobs after strict identity checks."""

    analysis = analyze_workflow(workflow, reusable_workflows)
    return {job_id: spec.exact_names for job_id, spec in analysis.reusable.items()}


def _reusable_job_ids_from_analysis(
    analysis: WorkflowIdentity,
    jobs: Iterable[Mapping[str, Any]],
    prefix: str,
) -> tuple[str, ...]:
    jobs = list(jobs)
    names: list[str] = []
    for job in jobs:
        name = job.get("name")
        if not isinstance(name, str) or not name:
            raise WorkflowIdentityError("api_job_name_missing")
        names.append(name)
    counts = Counter(names)
    if any(count > 1 for count in counts.values()):
        raise WorkflowIdentityError("api_duplicate_job_name")
    by_name = {str(job["name"]): job for job in jobs}
    reusable: list[str] = []
    for job_id, spec in analysis.reusable.items():
        expected = tuple(f"{prefix}{name}" for name in spec.exact_names)
        if spec.prefix:
            group_prefix = f"{prefix}{spec.prefix}"
            matching = [name for name in names if name.startswith(group_prefix)]
            unexpected = set(matching) - set(expected)
            # A leaf disabled by static inputs (e.g. the ordinary-only billing
            # smoke under coverage) is listed by GitHub as skipped. Accept it
            # only with that exact name and a ``skipped`` conclusion.
            skipped = {f"{prefix}{name}" for name in spec.skipped_names}
            unexpected -= {
                name for name in unexpected
                if name in skipped and by_name[name].get("conclusion") == "skipped"
            }
            if unexpected:
                raise WorkflowIdentityError("api_unexpected_reusable_job")
        if any(counts.get(name, 0) > 1 for name in expected):
            raise WorkflowIdentityError("api_duplicate_expected_job")
        if not expected or any(counts.get(name, 0) != 1 for name in expected):
            continue
        if all(
            by_name[name].get("status") == "completed"
            and by_name[name].get("conclusion") == "success"
            for name in expected
        ):
            reusable.append(job_id)
    return tuple(sorted(reusable))


def reusable_job_ids(
    workflow: Mapping[str, Any],
    jobs: Iterable[Mapping[str, Any]],
    prefix: str = "",
    reusable_workflows: Mapping[str, Mapping[str, Any]] | None = None,
) -> tuple[str, ...]:
    """Return top-level jobs whose complete exact observed job set succeeded."""

    analysis = analyze_workflow(workflow, reusable_workflows)
    return _reusable_job_ids_from_analysis(analysis, jobs, prefix)


def assess_tree_and_jobs(
    *,
    workflow: Mapping[str, Any],
    current_tree: str,
    tested_tree: str,
    jobs: Iterable[Mapping[str, Any]],
    source: str,
    run: Mapping[str, Any],
    head_sha: str | None = None,
    prefix: str = "",
    reusable_workflows: Mapping[str, Mapping[str, Any]] | None = None,
) -> Decision:
    jobs = list(jobs)
    run_jobs = tuple(sorted(str(job.get("name") or "") for job in jobs))
    if current_tree != tested_tree:
        return _fallback(
            "tree_mismatch", source, run_id=run.get("id"), run_url=run.get("html_url"),
            head_sha=head_sha, current_tree=current_tree, tested_tree=tested_tree,
            run_jobs=run_jobs,
        )
    try:
        analysis = analyze_workflow(workflow, reusable_workflows)
        reusable = _reusable_job_ids_from_analysis(analysis, jobs, prefix)
    except WorkflowIdentityError as exc:
        return _fallback(
            exc.reason, source, run_id=run.get("id"), run_url=run.get("html_url"),
            head_sha=head_sha, current_tree=current_tree, tested_tree=tested_tree,
            run_jobs=run_jobs,
        )
    if not reusable:
        return _fallback(
            "no_successful_jobs", source, run_id=run.get("id"), run_url=run.get("html_url"),
            head_sha=head_sha, current_tree=current_tree, tested_tree=tested_tree,
            candidates=analysis.candidates, run_jobs=run_jobs,
        )
    return Decision(
        True, "tree_match", source, run_id=run.get("id"), run_url=run.get("html_url"),
        head_sha=head_sha, current_tree=current_tree, tested_tree=tested_tree,
        candidates=analysis.candidates, jobs=reusable, run_jobs=run_jobs,
    )


def _run_jobs(api: Callable[[str], Any], run_id: int) -> list[Mapping[str, Any]]:
    pages = api(f"repos/{{repo}}/actions/runs/{run_id}/jobs?per_page=100")
    if isinstance(pages, Mapping):
        return list(pages.get("jobs") or [])
    return [job for page in pages for job in (page.get("jobs") or [])]


def _exactly_one_pr(value: Any, reason: str) -> Mapping[str, Any]:
    if not isinstance(value, list) or len(value) != 1 or not isinstance(value[0], Mapping):
        raise WorkflowIdentityError(reason)
    return value[0]


def _validated_merged_main_pr(
    pr: Mapping[str, Any], repository: str, merge_sha: str, head_sha: str
) -> tuple[int, str]:
    number = pr.get("number")
    base = pr.get("base")
    head = pr.get("head")
    if not isinstance(number, int) or not isinstance(base, Mapping) or not isinstance(head, Mapping):
        raise WorkflowIdentityError("main_pr_metadata_missing")
    base_repo = base.get("repo")
    head_repo = head.get("repo")
    if (base.get("ref") != "main" or not isinstance(base_repo, Mapping)
            or base_repo.get("full_name") != repository):
        raise WorkflowIdentityError("main_pr_base_mismatch")
    if not pr.get("merged_at") or pr.get("merge_commit_sha") != merge_sha:
        raise WorkflowIdentityError("main_pr_merge_mismatch")
    if head.get("sha") != head_sha or not isinstance(head_repo, Mapping):
        raise WorkflowIdentityError("main_pr_head_mismatch")
    full_name = head_repo.get("full_name")
    if not isinstance(full_name, str) or not full_name:
        raise WorkflowIdentityError("main_pr_head_repository_missing")
    return number, full_name


def decide_from_api(
    *,
    api: Callable[[str], Any],
    git_tree: Callable[[str], str],
    git_parent: Callable[[str], str] | None = None,
    git_is_ancestor: Callable[[str, str], bool] | None = None,
    git_blob: Callable[[str, str], str] | None = None,
    workflow_blob: str | None = None,
    workflow: Mapping[str, Any],
    repository: str,
    sha: str,
    event: str,
    ref: str,
    reusable_workflows: Mapping[str, Mapping[str, Any]] | None = None,
) -> Decision:
    """Resolve a publish event using injected API/git functions."""

    if event == "workflow_dispatch":
        return _fallback("workflow_dispatch", "none")

    current_tree: str | None = None
    try:
        current_tree = git_tree(sha)
        if git_blob is None or workflow_blob is None:
            return _fallback("workflow_blob_unavailable", "none", current_tree=current_tree)
        if git_blob(sha, WORKFLOW_PATH) != workflow_blob:
            return _fallback("workflow_blob_mismatch", "none", current_tree=current_tree)
        if event != "push":
            return _fallback("unsupported_event", "none", current_tree=current_tree)

        if ref.startswith("refs/tags/v"):
            runs = api(
                f"repos/{repository}/actions/workflows/publish-images.yml/runs"
                f"?event=push&head_sha={sha}&per_page=100"
            )
            runs = runs.get("workflow_runs", []) if isinstance(runs, Mapping) else runs
            candidates = [
                run for run in runs
                if run.get("head_sha") == sha and run.get("head_branch") == "main"
            ]
            if not candidates:
                return _fallback("main_publish_gate_missing", "tag", current_tree=current_tree)
            candidates.sort(
                key=lambda run: (
                    int(run.get("run_number") or 0),
                    _parse_time(run.get("updated_at") or run.get("created_at")),
                ),
                reverse=True,
            )
            run = candidates[0]
            if run.get("status") != "completed" or run.get("conclusion") != "success":
                return _fallback("main_publish_gate_missing", "tag", current_tree=current_tree)
            jobs = _run_jobs(api, int(run["id"]))
            gate_jobs = [job for job in jobs if str(job.get("name")) == f"{CI_GATE_PREFIX}{CI_PIPELINE_NAME}"]
            if len(gate_jobs) != 1 or any(job.get("conclusion") != "success" for job in gate_jobs):
                return _fallback("main_publish_gate_missing", "tag", current_tree=current_tree)
            return assess_tree_and_jobs(
                workflow=workflow, reusable_workflows=reusable_workflows,
                current_tree=current_tree, tested_tree=git_tree(sha), jobs=jobs,
                source="tag_main_publish", run=run, head_sha=sha, prefix=CI_GATE_PREFIX,
            )

        if ref != "refs/heads/main":
            return _fallback("unsupported_push_ref", "none", current_tree=current_tree)

        pr_m = _exactly_one_pr(
            api(f"repos/{repository}/commits/{sha}/pulls"), "main_commit_pr_ambiguous"
        )
        head = pr_m.get("head")
        if not isinstance(head, Mapping) or not isinstance(head.get("sha"), str):
            return _fallback("pull_request_head_missing", "main", current_tree=current_tree)
        head_sha = head["sha"]
        pr_h = _exactly_one_pr(
            api(f"repos/{repository}/commits/{head_sha}/pulls"), "head_commit_pr_ambiguous"
        )
        number_m, head_repository = _validated_merged_main_pr(pr_m, repository, sha, head_sha)
        number_h, head_repository_h = _validated_merged_main_pr(pr_h, repository, sha, head_sha)
        if number_m != number_h or head_repository != head_repository_h:
            return _fallback("associated_pr_mismatch", "main", current_tree=current_tree,
                             head_sha=head_sha)

        runs_response = api(
            f"repos/{repository}/actions/workflows/ci.yml/runs"
            f"?event=pull_request&head_sha={head_sha}&per_page=100"
        )
        runs = runs_response.get("workflow_runs", []) if isinstance(runs_response, Mapping) else runs_response
        runs = [run for run in (runs or []) if run.get("event") == "pull_request"]
        run = newest_completed_successful_run(runs or [], head_sha)
        if run is None:
            return _fallback("successful_pr_run_missing", "main", current_tree=current_tree,
                             head_sha=head_sha)
        if run.get("path") != WORKFLOW_PATH:
            return _fallback("workflow_path_mismatch", "main", current_tree=current_tree,
                             head_sha=head_sha)
        run_head_repo = (run.get("head_repository") or {}).get("full_name")
        if run_head_repo != head_repository:
            return _fallback("run_head_repository_mismatch", "main", current_tree=current_tree,
                             head_sha=head_sha)
        run_pull_requests = run.get("pull_requests") or []
        # Merged runs commonly expose an empty array. The two exact-one commit
        # lookups above prove the PR/base/merge identity in that case.
        if run_pull_requests:
            if len(run_pull_requests) != 1 or not isinstance(run_pull_requests[0], Mapping):
                return _fallback("run_pull_request_ambiguous", "main", current_tree=current_tree,
                                 head_sha=head_sha)
            run_pr = run_pull_requests[0]
            run_base = run_pr.get("base") or {}
            run_head = run_pr.get("head") or {}
            if (run_pr.get("number") != number_m or run_base.get("ref") != "main"
                    or run_head.get("sha") != head_sha):
                return _fallback("run_pull_request_mismatch", "main", current_tree=current_tree,
                                 head_sha=head_sha)
        if git_parent is None or git_is_ancestor is None:
            return _fallback("git_history_unavailable", "main", current_tree=current_tree,
                             head_sha=head_sha)
        parent_sha = git_parent(sha)
        if not git_is_ancestor(parent_sha, head_sha):
            return _fallback("pr_behind_main", "main", current_tree=current_tree,
                             head_sha=head_sha)
        jobs = _run_jobs(api, int(run["id"]))
        tested_tree = git_tree(head_sha)
        return assess_tree_and_jobs(
            workflow=workflow, reusable_workflows=reusable_workflows,
            current_tree=current_tree, tested_tree=tested_tree, jobs=jobs,
            source="pull_request", run=run, head_sha=head_sha,
        )
    except WorkflowIdentityError as exc:
        return _fallback(exc.reason, "main", current_tree=current_tree)
    except Exception as exc:  # Any API/git ambiguity fails open to full CI.
        return _fallback(f"lookup_error:{type(exc).__name__}", "error")


class GitHubApi:
    def __init__(self, repository: str):
        self.repository = repository

    def __call__(self, path: str) -> Any:
        path = path.replace("{repo}", self.repository)
        command = ["gh", "api", "--paginate", "--slurp", path]
        result = subprocess.run(command, check=True, capture_output=True, text=True)
        pages = json.loads(result.stdout)
        if not isinstance(pages, list) or len(pages) == 1:
            return pages[0] if isinstance(pages, list) and pages else pages
        if all(isinstance(page, list) for page in pages):
            return [item for page in pages for item in page]
        if all(isinstance(page, Mapping) for page in pages):
            merged: dict[str, Any] = dict(pages[0])
            for page in pages[1:]:
                for key, value in page.items():
                    if isinstance(value, list) and isinstance(merged.get(key), list):
                        merged[key] = [*merged[key], *value]
                    elif key not in merged:
                        merged[key] = value
            return merged
        return pages


class GitObjects:
    def __call__(self, commit: str) -> str:
        self._fetch(commit)
        return self._run("rev-parse", f"{commit}^{{tree}}")

    def parent(self, commit: str) -> str:
        self._fetch(commit)
        return self._run("rev-parse", f"{commit}^1")

    def blob(self, commit: str, path: str) -> str:
        self._fetch(commit)
        return self._run("rev-parse", f"{commit}:{path}")

    def file_blob(self, path: str) -> str:
        return self._run("hash-object", "--", path)

    def is_ancestor(self, ancestor: str, descendant: str) -> bool:
        self._fetch(ancestor, descendant)
        result = subprocess.run(
            ["git", "merge-base", "--is-ancestor", ancestor, descendant],
            capture_output=True, text=True,
        )
        if result.returncode == 0:
            return True
        if result.returncode == 1:
            return False
        raise subprocess.CalledProcessError(result.returncode, result.args, result.stdout, result.stderr)

    @classmethod
    def _fetch(cls, *commits: str) -> None:
        missing = []
        for commit in commits:
            result = subprocess.run(
                ["git", "cat-file", "-e", f"{commit}^{{commit}}"],
                capture_output=True, text=True,
            )
            if result.returncode != 0:
                missing.append(commit)
        if missing:
            cls._run("fetch", "--no-tags", "origin", *missing)

    @staticmethod
    def _run(*args: str) -> str:
        result = subprocess.run(["git", *args], check=True, capture_output=True, text=True)
        return result.stdout.strip()


def _load_workflow_bundle(root: Path, entry: str) -> tuple[Mapping[str, Mapping[str, Any]], str]:
    workflows: dict[str, Mapping[str, Any]] = {}
    texts: dict[str, str] = {}

    def load(path: str) -> None:
        path = PurePosixPath(path).as_posix()
        if path in workflows:
            return
        file_path = root / path
        if not file_path.is_file():
            raise WorkflowSchemaError("workflow_file_missing")
        text = file_path.read_text(encoding="utf-8")
        workflow = strict_load_workflow(text)
        workflows[path] = workflow
        texts[path] = text
        jobs = workflow.get("jobs") or {}
        if not isinstance(jobs, Mapping):
            raise WorkflowIdentityError("workflow_identity_jobs_mapping")
        for job in jobs.values():
            if isinstance(job, Mapping) and "uses" in job:
                uses = job.get("uses")
                if isinstance(uses, str) and uses.startswith("./"):
                    child = (uses[2:] if uses.startswith("./.github/") else
                             (PurePosixPath(path).parent / uses[2:]).as_posix())
                    if child.startswith("../"):
                        raise WorkflowIdentityError("workflow_identity_reusable_path")
                    load(child)

    load(entry)
    entry_text = texts[entry]
    return workflows, entry_text


def _write_summary(decision: Decision, path: str) -> None:
    payload = decision.payload()
    reused = set(decision.jobs)
    lines = [
        "## CI gate reuse decision",
        f"- Decision: **{'reuse eligible jobs' if decision.reuse else 'run full gate'}** ({decision.reason})",
        f"- Source: `{decision.source}`; run: `{decision.run_id or 'none'}`",
        f"- Trees: current `{decision.current_tree or 'unknown'}`, tested `{decision.tested_tree or 'unknown'}`",
        f"- Reused top-level jobs: {', '.join(f'`{job}`' for job in sorted(reused)) or 'none'}",
        f"- Main-run candidates: {', '.join(f'`{job}`' for job in decision.candidates if job not in reused) or 'none'}",
    ]
    with open(path, "a", encoding="utf-8") as summary:
        summary.write("\n".join(lines) + "\n")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--event", required=True)
    parser.add_argument("--ref", required=True)
    parser.add_argument("--workflow", default=WORKFLOW_PATH)
    parser.add_argument("--summary")
    args = parser.parse_args(argv)
    try:
        if args.workflow != WORKFLOW_PATH:
            raise WorkflowSchemaError("workflow_path_unexpected")
        workflows, _ = _load_workflow_bundle(Path("."), args.workflow)
        workflow = workflows[args.workflow]
    except (WorkflowSchemaError, WorkflowIdentityError) as exc:
        decision = _fallback(exc.reason, "error")
        workflows = None
    except Exception as exc:
        decision = _fallback(f"workflow_read_error:{type(exc).__name__}", "error")
        workflows = None
    else:
        git = GitObjects()
        try:
            workflow_blob = git.file_blob(args.workflow)
        except Exception as exc:
            decision = _fallback(f"workflow_blob_read_error:{type(exc).__name__}", "error")
        else:
            decision = decide_from_api(
                api=GitHubApi(args.repository),
                git_tree=git,
                git_parent=git.parent,
                git_is_ancestor=git.is_ancestor,
                git_blob=git.blob,
                workflow_blob=workflow_blob,
                workflow=workflow,
                repository=args.repository,
                sha=args.sha,
                event=args.event,
                ref=args.ref,
                reusable_workflows=workflows,
            )
    if args.summary:
        _write_summary(decision, args.summary)
    print(json.dumps(decision.payload(), separators=(",", ":")))
    return 0


if __name__ == "__main__":
    sys.exit(main())

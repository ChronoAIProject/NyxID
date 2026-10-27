#!/usr/bin/env python3
"""Decide whether a publish CI gate can reuse an identical PR/main run.

The decision is deliberately conservative.  GitHub metadata identifies the
candidate run and its jobs; git objects identify the tree tested by that run.
No artifact or PR-produced text participates in the decision.
"""

from __future__ import annotations

import argparse
import itertools
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping


WORKFLOW_PATH = ".github/workflows/ci.yml"
PUBLISH_WORKFLOW_PATH = ".github/workflows/publish-images.yml"
CI_PIPELINE_NAME = "CI Pipeline"
CI_GATE_PREFIX = "CI Gate / "


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
    """Return the newest attempt, only when that attempt completed successfully.

    A newer failed or in-progress attempt invalidates an older successful one.
    This prevents reuse after a re-run has not passed yet.
    """

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


def _matrix_rows(strategy: Mapping[str, Any] | None) -> list[Mapping[str, Any]] | None:
    if not strategy or not isinstance(strategy.get("matrix"), Mapping):
        return [{}]
    matrix = strategy["matrix"]
    include = matrix.get("include")
    if isinstance(include, list):
        return [row for row in include if isinstance(row, Mapping)]
    keys = [key for key, value in matrix.items() if isinstance(value, list)]
    if not keys:
        return None
    return [dict(zip(keys, values)) for values in itertools.product(*(matrix[key] for key in keys))]


_MATRIX_EXPRESSION = re.compile(r"\$\{\{\s*matrix\.([A-Za-z_][A-Za-z0-9_]*)\s*\}\}")


def _has_event_sensitive_steps(job: Mapping[str, Any]) -> bool:
    """Keep jobs with event/force-all-dependent steps on the main run.

    A successful PR job can still have skipped steps (for example the Claude
    package checks). Those jobs must run on main even when the job itself
    reported success. Coverage threshold jobs are handled separately because
    their backend baseline publication is deliberately non-blocking.
    """

    details = {key: value for key, value in job.items()
               if key not in {"name", "if", "needs", "strategy", "uses"}}
    text = json.dumps(details, sort_keys=True)
    return any(marker in text for marker in ("github.event_name", "github.ref", "inputs.force-all"))


def _job_name(job_id: str, job: Mapping[str, Any], row: Mapping[str, Any]) -> str:
    name = str(job.get("name") or job_id)
    return _MATRIX_EXPRESSION.sub(lambda match: str(row.get(match.group(1), match.group(0))), name)


def expected_job_names(workflow: Mapping[str, Any]) -> dict[str, tuple[str, ...] | str]:
    """Map CI job IDs to their API names.

    A reusable workflow is represented by all child jobs under ``name /`` in
    the caller's jobs API.  Its value is therefore a prefix rather than one
    exact name.  Matrix jobs expand to every declared matrix value so a
    partial matrix can never be reused.
    """

    required = set((workflow.get("jobs", {}).get("ci-pass", {}).get("needs") or []))
    result: dict[str, tuple[str, ...] | str] = {}
    for job_id, job in (workflow.get("jobs") or {}).items():
        # Only jobs in the final pipeline and explicitly guarded by this
        # feature can be reused.  This excludes PR-only informational jobs and
        # keeps the reuse list aligned with the actual `if:` conditions.
        if (job_id not in required
                or "inputs.reuse-jobs" not in str(job.get("if") or "")):
            continue
        name = str(job.get("name") or job_id)
        if job.get("uses"):
            result[job_id] = f"{name} / "
            continue
        rows = _matrix_rows(job.get("strategy"))
        if rows is None:
            # Unknown matrix construction is safer to run again.
            result[job_id] = ()
        else:
            result[job_id] = tuple(_job_name(job_id, job, row) for row in rows)
    return result


def reusable_job_ids(
    workflow: Mapping[str, Any], jobs: Iterable[Mapping[str, Any]], prefix: str = ""
) -> tuple[str, ...]:
    """Return top-level jobs whose complete observed job set succeeded."""

    jobs = list(jobs)
    observed = [
        job
        for job in jobs
        if job.get("status") == "completed"
        and job.get("conclusion") == "success"
    ]
    names = [str(job.get("name") or "") for job in jobs]
    successful_names = {str(job.get("name") or "") for job in observed}
    reusable: list[str] = []
    for job_id, expected in expected_job_names(workflow).items():
        definition = (workflow.get("jobs") or {}).get(job_id, {})
        # Coverage threshold jobs measure the same head tree. Their
        # event-specific baseline/report publication is handled separately (the
        # backend baseline producer is outside the blocking CI Gate).
        if _has_event_sensitive_steps(definition) and not job_id.startswith("coverage-"):
            continue
        if isinstance(expected, str):
            matches = [name for name in names if name.startswith(prefix + expected)]
            if matches and all(name in successful_names for name in matches):
                reusable.append(job_id)
        elif expected and all(prefix + name in successful_names for name in expected):
            reusable.append(job_id)
    return tuple(sorted(reusable))


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
) -> Decision:
    jobs = list(jobs)
    run_jobs = tuple(sorted(str(job.get("name") or "") for job in jobs))
    if current_tree != tested_tree:
        return _fallback(
            "tree_mismatch",
            source,
            run_id=run.get("id"),
            run_url=run.get("html_url"),
            head_sha=head_sha,
            current_tree=current_tree,
            tested_tree=tested_tree,
            run_jobs=run_jobs,
        )
    candidates = tuple(sorted(expected_job_names(workflow)))
    reusable = reusable_job_ids(workflow, jobs, prefix=prefix)
    if not reusable:
        return _fallback(
            "no_successful_jobs",
            source,
            run_id=run.get("id"),
            run_url=run.get("html_url"),
            head_sha=head_sha,
            current_tree=current_tree,
            tested_tree=tested_tree,
            candidates=candidates,
            run_jobs=run_jobs,
        )
    return Decision(
        True,
        "tree_match",
        source,
        run_id=run.get("id"),
        run_url=run.get("html_url"),
        head_sha=head_sha,
        current_tree=current_tree,
        tested_tree=tested_tree,
        candidates=candidates,
        jobs=reusable,
        run_jobs=run_jobs,
    )


def _run_jobs(api: Callable[[str], Any], run_id: int) -> list[Mapping[str, Any]]:
    pages = api(f"repos/{{repo}}/actions/runs/{run_id}/jobs?per_page=100")
    if isinstance(pages, Mapping):
        return list(pages.get("jobs") or [])
    return [job for page in pages for job in (page.get("jobs") or [])]


def decide_from_api(
    *,
    api: Callable[[str], Any],
    git_tree: Callable[[str], str],
    git_parent: Callable[[str], str] | None = None,
    git_is_ancestor: Callable[[str, str], bool] | None = None,
    workflow: Mapping[str, Any],
    repository: str,
    sha: str,
    event: str,
    ref: str,
) -> Decision:
    """Resolve a publish event using injected API/git functions."""

    if event == "workflow_dispatch":
        return _fallback("workflow_dispatch", "none")

    try:
        current_tree = git_tree(sha)
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
            if not gate_jobs or any(job.get("conclusion") != "success" for job in gate_jobs):
                return _fallback("main_publish_gate_missing", "tag", current_tree=current_tree)
            return assess_tree_and_jobs(
                workflow=workflow,
                current_tree=current_tree,
                tested_tree=git_tree(sha),
                jobs=jobs,
                source="tag_main_publish",
                run=run,
                head_sha=sha,
                prefix=CI_GATE_PREFIX,
            )

        if ref != "refs/heads/main":
            return _fallback("unsupported_push_ref", "none", current_tree=current_tree)

        prs = api(f"repos/{repository}/commits/{sha}/pulls")
        if not isinstance(prs, list) or len(prs) != 1:
            return _fallback("ambiguous_pull_request", "main", current_tree=current_tree)
        pr = prs[0]
        head_sha = str((pr.get("head") or {}).get("sha") or "")
        if not head_sha:
            return _fallback("pull_request_head_missing", "main", current_tree=current_tree)
        runs_response = api(
            f"repos/{repository}/actions/workflows/ci.yml/runs"
            f"?event=pull_request&head_sha={head_sha}&per_page=100"
        )
        runs = runs_response.get("workflow_runs", []) if isinstance(runs_response, Mapping) else runs_response
        runs = [run for run in (runs or []) if run.get("event") == "pull_request"]
        run = newest_completed_successful_run(runs or [], head_sha)
        if run is None:
            return _fallback("successful_pr_run_missing", "main", current_tree=current_tree)
        # The Actions API normally includes ``path``.  Treat an omitted path
        # as ambiguous rather than accepting a run from an unknown workflow.
        if run.get("path") != WORKFLOW_PATH:
            return _fallback("workflow_path_mismatch", "main", current_tree=current_tree)
        run_pull_requests = run.get("pull_requests") or []
        # The commit-to-PR lookup above is the authoritative unique
        # association. GitHub often empties the run's copy after merge; when it
        # is present, validate that it still names that same PR/head.
        if run_pull_requests:
            if len(run_pull_requests) != 1:
                return _fallback("run_pull_request_ambiguous", "main", current_tree=current_tree)
            run_pr = run_pull_requests[0]
            if (run_pr.get("number") is not None and pr.get("number") is not None
                    and run_pr.get("number") != pr.get("number")):
                return _fallback("run_pull_request_mismatch", "main", current_tree=current_tree)
            run_head = (run_pr.get("head") or {}).get("sha")
            if run_head and run_head != head_sha:
                return _fallback("run_head_mismatch", "main", current_tree=current_tree)
        if git_parent is None or git_is_ancestor is None:
            return _fallback("git_history_unavailable", "main", current_tree=current_tree)
        parent_sha = git_parent(sha)
        if not git_is_ancestor(parent_sha, head_sha):
            return _fallback("pr_behind_main", "main", current_tree=current_tree,
                             head_sha=head_sha)
        jobs = _run_jobs(api, int(run["id"]))
        tested_tree = git_tree(head_sha)
        return assess_tree_and_jobs(
            workflow=workflow,
            current_tree=current_tree,
            tested_tree=tested_tree,
            jobs=jobs,
            source="pull_request",
            run=run,
            head_sha=head_sha,
        )
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

    def is_ancestor(self, ancestor: str, descendant: str) -> bool:
        self._fetch(ancestor, descendant)
        result = subprocess.run(
            ["git", "merge-base", "--is-ancestor", ancestor, descendant],
            capture_output=True,
            text=True,
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
                capture_output=True,
                text=True,
            )
            if result.returncode != 0:
                missing.append(commit)
        if missing:
            cls._run("fetch", "--no-tags", "origin", *missing)

    @staticmethod
    def _run(*args: str) -> str:
        result = subprocess.run(["git", *args], check=True, capture_output=True, text=True)
        return result.stdout.strip()


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
        import yaml

        workflow = yaml.safe_load(Path(args.workflow).read_text(encoding="utf-8"))
    except Exception as exc:
        decision = _fallback(f"workflow_read_error:{type(exc).__name__}", "error")
    else:
        git = GitObjects()
        decision = decide_from_api(
            api=GitHubApi(args.repository),
            git_tree=git,
            git_parent=git.parent,
            git_is_ancestor=git.is_ancestor,
            workflow=workflow,
            repository=args.repository,
            sha=args.sha,
            event=args.event,
            ref=args.ref,
        )
    if args.summary:
        _write_summary(decision, args.summary)
    print(json.dumps(decision.payload(), separators=(",", ":")))
    return 0


if __name__ == "__main__":
    sys.exit(main())

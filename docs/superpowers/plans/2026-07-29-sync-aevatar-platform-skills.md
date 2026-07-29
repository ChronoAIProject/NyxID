# Sync Aevatar Platform Skills Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace NyxID’s stale bundled Aevatar skill snapshot with the exact 15-package public closure of `aevatar-platform@1.14`, validate it, and open a pull request to `ChronoAIProject/NyxID:main`.

**Architecture:** Treat Ornn immutable ZIPs and closure SHA-256 values as content authority. Verify outside the repository, mechanically mirror each package into `skills/<name>/`, then update only plugin distribution metadata and installation documentation.

**Tech Stack:** Git, NyxID CLI proxy, Ornn `/api/v1`, ZIP, SHA-256, Ruby, jq, shell.

## Global Constraints

- Pin skillset GUID `248b99d6-36ff-4d41-bb45-baa25c6a9cad` and version `1.14`.
- Preserve all 15 exact name/version/hash rows from the pinned closure.
- Select the live Ornn UserService from typed inventory and pass its exact ID with `--via-service`.
- Never print or commit credentials.
- Preserve published business instructions; host adapters are limited to two equivalent trigger descriptions, node-setup invocation safety, and required JSON `Content-Type` guidance for NyxID proxy writes.
- Do not change NyxID runtime code or add dependencies/updaters.
- Set the Claude, Codex, Cursor, and Claude marketplace manifest versions to `0.7.0`.

---

### Task 1: Freeze Authority and Prove RED

**Files:**
- Read: `skills/*/SKILL.md`
- Create outside repo: `/tmp/nyxid-aevatar-platform-1.14/**`

**Interfaces:**
- Consumes: NyxID service inventory and pinned skillset.
- Produces: validated `closure.json` and exact `ORNN_USER_SERVICE_ID`.

- [x] Compare repository Aevatar/fallback skill names with the 15 closure names.

Expected: FAIL with seven missing packages.

- [x] Select exactly one active connected UserService row with `slug=ornn-api` and endpoint host `ornn-api-deployment-api-svc.chronoai-platform.svc.cluster.local` from `nyxid service list --output json`.

- [x] Read `/api/v1/skillsets/aevatar-platform/closure?version=1.14` through that exact binding.

Assert `error == null`, 15 unique names, expected versions, and 64-character hashes.

### Task 2: Mirror the Immutable Packages

**Files:**
- Replace: existing `skills/aevatar-*/**` and `skills/fallback-to-calling-agent/**` package roots.
- Add: missing `skills/aevatar-*/**`, `skills/firecrawl-via-nyxid/**`, and `skills/github-via-nyxid/**` roots.

**Interfaces:**
- Consumes: exact closure rows.
- Produces: nine byte-equal package directories plus six narrowly adapted host-compatible directories.

- [x] Download each `/api/v1/skills/<name>/versions/<version>/download` ZIP outside the repository.

- [x] Run `unzip -t` and `shasum -a 256`; stop before copying on mismatch.

- [x] Require one `<name>/SKILL.md` root per ZIP, then mechanically mirror it with deletion to `skills/<name>/`.

- [x] For `aevatar-codex-exec-node-setup`, set `disable-model-invocation: false`, add `agents/openai.yaml` with `policy.allow_implicit_invocation: false`, and add an explicit-request body gate before any mutation.

- [x] Compress only scheduler/profile descriptions to at most 1,024 characters without changing their trigger boundaries.

- [x] Keep the published versions and add JSON `Content-Type` only to the NyxID proxy write guidance in team-builder, workflow-authoring, and platform-map; the CLI otherwise sends `-d` as `application/octet-stream`.

- [x] Recursively compare nine roots with verified extractions. Require only the declared frontmatter/body/metadata adaptations in the other six. Check all names and versions.

### Task 3: Align Plugin Metadata

**Files:**
- Modify: `skills/INSTALL.md`
- Modify: `.codex-plugin/plugin.json`
- Modify: `.claude-plugin/plugin.json`
- Modify: `.claude-plugin/marketplace.json`
- Modify: `.cursor-plugin/plugin.json`
- Modify only if routing requires it: `skills/nyxid/SKILL.md`

**Interfaces:**
- Consumes: final 15-package bundle.
- Produces: accurate installation docs and four version `0.7.0` distribution manifests.

- [x] Document all bundled Aevatar skills and `aevatar-platform-map` as the local router.

- [x] Set four manifest versions to `0.7.0` and mention the bundled Aevatar family concisely.

- [x] Keep NyxID trigger scope unchanged; route explicit Aevatar requests to the map only if current text lacks it.

### Task 4: Verify GREEN

**Files:**
- Test: all changed `skills/**` and manifests.

**Interfaces:**
- Consumes: synchronized bundle and metadata.
- Produces: local verification evidence.

- [x] Re-run the inventory assertion; require exactly 15 expected names and versions.

- [x] Re-run recursive package comparison and all ZIP hash checks.

- [x] Run `quick_validate.py` for each package; classify only published optional-frontmatter incompatibilities.

- [x] Run:

```bash
ruby skills/nyxid/evals/run_trigger_evals.rb --validate-only
jq empty .codex-plugin/plugin.json .claude-plugin/plugin.json .claude-plugin/marketplace.json .cursor-plugin/plugin.json
git diff --check
```

- [x] Search active instructions for identity inference, global lifecycle, obsolete `SkillRunner`, flattened readiness, raw stored-token guidance, and stale tool names; inspect every match.

- [x] Review the complete diff and reject unrelated application changes.

Verification evidence (2026-07-29):

- Immutable packages: 15/15 SHA-256 matches and 15/15 archives pass `unzip -t`.
- Repository closure: 15/15 names and versions; 9/9 byte-equal roots; 6/6 declared host adaptations only.
- Plugin checks: 12/12 NyxID trigger cases; four valid JSON manifests at `0.7.0`; Codex manifest schema check; Claude marketplace and root-free plugin mirror strict validation.
- Frontend baseline: 2,100 tests pass; lint exits 0 with 23 existing warnings.
- Rust baseline: representative formerly failing MongoDB-backed tests pass 3/3 after cleanup. Full `cargo test -- --test-threads=1` is not green: the existing test helper creates a database per test and defers cleanup until process exit, accumulating hundreds of `nyxid_test_*` databases and causing unrelated MongoDB-backed tests to fail. No Rust source is changed by this branch.
- Ornn format validation: 8/15 live validations passed; the remaining seven requests ended in repeated TLS EOF. Their immutable ZIP hashes, archive integrity, repository readback, names, and versions all pass locally.

### Task 5: Commit and Open the PR

**Files:**
- Commit: validated scoped changes only.

**Interfaces:**
- Consumes: Task 4 evidence.
- Produces: pushed branch and PR to `ChronoAIProject/NyxID:main`.

- [x] Fetch upstream main; rebase and repeat Task 4 if it advanced.

- [x] Stage scoped files, run cached diff checks, and commit with an imperative message.

- [x] Push `feat/2026-07-29_sync-aevatar-platform-skills` to `eanz17/NyxID`.

- [x] Create the PR with authority, closure delta, paths, validation results, and no-runtime-change statement; read it back before reporting.

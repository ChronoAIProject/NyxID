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
- Do not hand-edit published package content.
- Do not change NyxID runtime code or add dependencies/updaters.
- Set all three plugin manifest versions to `0.7.0`.

---

### Task 1: Freeze Authority and Prove RED

**Files:**
- Read: `skills/*/SKILL.md`
- Create outside repo: `/tmp/nyxid-aevatar-platform-1.14/**`

**Interfaces:**
- Consumes: NyxID service inventory and pinned skillset.
- Produces: validated `closure.json` and exact `ORNN_USER_SERVICE_ID`.

- [ ] Compare repository Aevatar/fallback skill names with the 15 closure names.

Expected: FAIL with seven missing packages.

- [ ] Select exactly one active connected UserService row with `slug=ornn-api` and endpoint host `ornn-api-deployment-api-svc.chronoai-platform.svc.cluster.local` from `nyxid service list --output json`.

- [ ] Read `/api/v1/skillsets/aevatar-platform/closure?version=1.14` through that exact binding.

Assert `error == null`, 15 unique names, expected versions, and 64-character hashes.

### Task 2: Mirror the Immutable Packages

**Files:**
- Replace: existing `skills/aevatar-*/**` and `skills/fallback-to-calling-agent/**` package roots.
- Add: missing `skills/aevatar-*/**`, `skills/firecrawl-via-nyxid/**`, and `skills/github-via-nyxid/**` roots.

**Interfaces:**
- Consumes: exact closure rows.
- Produces: 15 repository directories equal to published ZIP contents.

- [ ] Download each `/api/v1/skills/<name>/versions/<version>/download` ZIP outside the repository.

- [ ] Run `unzip -t` and `shasum -a 256`; stop before copying on mismatch.

- [ ] Require one `<name>/SKILL.md` root per ZIP, then mechanically mirror it with deletion to `skills/<name>/`.

- [ ] Recursively compare all repository roots with the verified extractions and check declared names/versions.

### Task 3: Align Plugin Metadata

**Files:**
- Modify: `skills/INSTALL.md`
- Modify: `.codex-plugin/plugin.json`
- Modify: `.claude-plugin/plugin.json`
- Modify: `.claude-plugin/marketplace.json`
- Modify only if routing requires it: `skills/nyxid/SKILL.md`

**Interfaces:**
- Consumes: final 15-package bundle.
- Produces: accurate installation docs and version `0.7.0` manifests.

- [ ] Document all bundled Aevatar skills and `aevatar-platform-map` as the local router.

- [ ] Set three manifest versions to `0.7.0` and mention the bundled Aevatar family concisely.

- [ ] Keep NyxID trigger scope unchanged; route explicit Aevatar requests to the map only if current text lacks it.

### Task 4: Verify GREEN

**Files:**
- Test: all changed `skills/**` and manifests.

**Interfaces:**
- Consumes: synchronized bundle and metadata.
- Produces: local verification evidence.

- [ ] Re-run the inventory assertion; require exactly 15 expected names and versions.

- [ ] Re-run recursive package comparison and all ZIP hash checks.

- [ ] Run `quick_validate.py` for each package; classify only published optional-frontmatter incompatibilities.

- [ ] Run:

```bash
ruby skills/nyxid/evals/run_trigger_evals.rb --validate-only
jq empty .codex-plugin/plugin.json .claude-plugin/plugin.json .claude-plugin/marketplace.json
git diff --check
```

- [ ] Search active instructions for identity inference, global lifecycle, obsolete `SkillRunner`, flattened readiness, raw stored-token guidance, and stale tool names; inspect every match.

- [ ] Review the complete diff and reject unrelated application changes.

### Task 5: Commit and Open the PR

**Files:**
- Commit: validated scoped changes only.

**Interfaces:**
- Consumes: Task 4 evidence.
- Produces: pushed branch and PR to `ChronoAIProject/NyxID:main`.

- [ ] Fetch upstream main; rebase and repeat Task 4 if it advanced.

- [ ] Stage scoped files, run cached diff checks, and commit with an imperative message.

- [ ] Push `feat/2026-07-29_sync-aevatar-platform-skills` to `eanz17/NyxID`.

- [ ] Create the PR with authority, closure delta, paths, validation results, and no-runtime-change statement; read it back before reporting.

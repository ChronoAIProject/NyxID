# Sync Aevatar Platform Skills into the NyxID Plugin

## Goal

Update the NyxID plugin's bundled Aevatar skills from the stale eight-skill snapshot to the exact public closure published as `aevatar-platform@1.14`, then open a pull request against `ChronoAIProject/NyxID:main`.

## Authority

- Aevatar source authority: the refreshed `aevatarAI/aevatar` `feature/integrate` contract used to publish the current skillset.
- Distribution authority: Ornn skillset GUID `248b99d6-36ff-4d41-bb45-baa25c6a9cad`, version `1.14`.
- Content authority: each exact Ornn ZIP and its SHA-256 from the skillset closure. Repository copies are derived artifacts, not independently authored variants.
- NyxID integration baseline: `ChronoAIProject/NyxID:main` at `1eaa1dac00560b782bf6091c6ec00cd6cc98a5d8`.

## Semantic Decision

The plugin will bundle the complete `aevatar-platform@1.14` closure so local Claude Code, Codex, Cursor, and compatible skill hosts use the same resource identities and tool contracts as Aevatar's published runtime manuals.

The synchronized skills must preserve these current semantics:

- `memberId`, draft `workflowId`, `publishedServiceId`, Agent Profile ID, UserService ID, operation ID, schedule ID, and Agent Key ID are separate identities.
- No global Build/Bind/Invoke/Observe lifecycle is inferred across Studio resources.
- Client mode routes Aevatar calls through the NyxID broker; in-session mode uses only tools actually present.
- Raw proxy, dynamic connected-service operation, and compiled admitted workflow operation remain distinct call shapes.
- Accepted/202 receipts prove admission only; committed or projected state requires authoritative readback.
- Skill-first discovery and typed readiness/admission contracts replace path, ID, or tool-schema guessing.

## Bundled Closure

The plugin will contain these exact roots:

| Skill | Version |
|---|---:|
| `fallback-to-calling-agent` | 1.0 |
| `aevatar-workflow-authoring` | 2.1 |
| `aevatar-team-builder` | 1.3 |
| `aevatar-scheduler` | 1.8 |
| `aevatar-service-publisher` | 1.5 |
| `aevatar-platform-map` | 1.10 |
| `aevatar-agent-profile-management` | 1.0 |
| `aevatar-feasibility-advisor` | 1.4 |
| `aevatar-triage` | 1.6 |
| `firecrawl-via-nyxid` | 1.2 |
| `github-via-nyxid` | 1.1 |
| `aevatar-automation` | 1.2 |
| `aevatar-channels-delivery` | 1.3 |
| `aevatar-codex-exec-workflow-sample` | 3.1 |
| `aevatar-codex-exec-node-setup` | 4.1 |

The closure hashes returned by Ornn are the verification ledger. A package whose downloaded hash does not match will not be copied or committed.

## Repository Changes

1. Replace the eight existing Aevatar skill directories with their exact published package contents.
2. Add the seven missing skill directories from the same closure.
3. Keep the generic `skills/nyxid` package independent; change it only if its routing map must mention the bundled Aevatar entry point.
4. Update `skills/INSTALL.md` so the documented bundle matches the installed directories.
5. Set all three plugin manifest versions to `0.7.0` and keep their descriptions aligned with the expanded Aevatar bundle.
6. Add the smallest deterministic verification script or test only if the repository lacks a native check for closure membership, declared versions, and manifest consistency.

No NyxID backend, frontend, CLI runtime, database, OAuth, proxy, or deployment behavior changes are in scope.

## Synchronization Flow

1. Read skillset `1.14` detail and closure through the caller's exact NyxID UserService binding.
2. For each closure row, download the declared immutable version from Ornn through NyxID.
3. Verify the ZIP SHA-256 against `skillHash`, verify the archive, and extract outside the repository.
4. Compare the extracted package root with the corresponding repository directory.
5. Copy only verified package files into `skills/<name>/`; delete files absent from the published package.
6. Update installation metadata and plugin manifests.
7. Run static checks, skill validation, and package/content readback checks before commit.

An ambiguous download or read failure is safe to retry as a read. No Ornn publication or other external mutation is required for this repository sync.

## Validation

- Prove the current stale snapshot fails an inventory check against the 15-member closure before changing files.
- Validate every downloaded ZIP and exact SHA-256.
- Assert the repository has exactly the 15 expected Aevatar/fallback roots at the declared versions after synchronization.
- Assert all three plugin manifests are valid JSON and declare `0.7.0`.
- Run `ruby skills/nyxid/evals/run_trigger_evals.rb --validate-only`.
- Run the skill creator validator for every changed or added skill when compatible with the published package format.
- Search the synchronized Aevatar skill set for stale identity equality, obsolete global lifecycle, old tool schemas, and raw stored-token guidance; manually inspect every match.
- Run `git diff --check` and the repository's relevant skill/CI checks.

Credentialed model trigger evaluation remains a trusted CI job on the pull request; no credential is copied into the repository or test artifacts.

## Failure Handling

- Hash mismatch: stop before copying that package and report the exact skill/version.
- Closure changes during work: reread the requested `1.14` closure; do not silently switch versions.
- Published package conflicts with current Aevatar source: treat it as a publication defect and stop rather than hand-editing a third variant.
- Upstream NyxID advances: rebase before final verification and repeat the relevant diff/validation checks.
- Ambiguous Git push or PR creation: read back the remote branch or PR before any retry.

## Pull Request Shape

Use one focused commit for the synchronized plugin artifacts and validation support after this design commit. The PR description will list the old and new closures, the authoritative skillset/version, changed paths, verification commands/results, and the fact that NyxID runtime behavior is unchanged.

## Deliberate Omissions

- No dynamic runtime Ornn dependency replaces the bundled skills.
- No generic synchronization framework, scheduled updater, or new dependency is introduced.
- No unrelated NyxID documentation or application code is refactored.
- No Aevatar internal implementation notes are copied unless they are already part of the published skill packages.

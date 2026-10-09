# Agent skills

## Storage and authority

`AssistantAgent.skills` contains at most 16 `SkillReference` entries, unique by
`(source, skill_id)`, beside `grants`. Only `ornn` is supported. Every root and
dependency uses a stable GUID, exact version and lowercase ZIP SHA-256.
`skills_revision` is an optimistic concurrency fence, initially zero.
Bounded description and archive-size metadata are stored separately; bodies
are never stored on agents. Missing fields mean no skills. Grant writers must
update their own fields only.

Ownership is independent of skill pins. Both NyxBot and specialists support
skills; B3 may replace personal ownership checks with maintainer ACLs without
changing the pin model. Search, preview, pin and reads use the managing/acting
person's live Ornn visibility through the `ornn-api` catalog integration.
The catalog integration must propagate a signed person identity (`jwt` or
`both`); it may use that person's connection or identity-only authentication.
Shared master credentials are forbidden, including legacy fallback and platform
bindings. Missing identity fails closed. No cross-person package cache exists.

## Packages and disclosure

Search uses Ornn's mixed-visibility keyword search. Version discovery and exact
metadata reads precede pinning. Dependency closure is resolved for that person;
each dependency is pinned and verified, with at most 16 dependencies per root.
Downloads use GUID and literal version, never a mutable name or dist-tag.
Verify SHA-256 over the raw ZIP before reading files. Bound compressed and
expanded bytes, entry count and paths; reject traversal, symlinks, duplicate
paths and malformed packages. Never extract to disk or execute scripts.

Turn instructions contain only bounded names, one-line descriptions and exact
versions, with an explicit untrusted-guidance notice. Guest turns omit skills.
`nyxid__skill_read` reads only the live calling agent's attached root or pinned
dependency, defaults to `SKILL.md`, and supports byte-offset paging at UTF-8
boundaries. Serialized results remain below 10,000 characters. A file listing
is available via `path: "/"`. Unavailable Ornn, missing access and integrity
failures produce bounded tool errors and never prevent a turn from starting.
Revocation/removal takes effect on the next read. The reader also carries any
configured Ornn operation scope from the already loaded agent into proxy
enforcement; attachment cannot bypass an explicit operation restriction. Other agents, guests, other
owners and non-assistant callers cannot use this reader.

Skills are external, untrusted guidance. They never alter grants, operation
scopes, approvals, models or machine permissions. Bundled scripts are text;
saving or running them requires existing tools and their existing permissions.

## Management

Owner UI provides Ornn search, version preview (description and byte size),
explicit attach/re-pin, removal and an available-update indication. Every
write carries the observed revision; stale writes conflict without rebasing.
NyxBot's `nyxid__set_agent_skills` removes immediately. Additions and re-pins
require a one-use owner action card bound to the target agent, revision and
exact complete pins, including dependency hashes. Skip-destructive never
bypasses this card. Pinning rechecks Ornn access and package integrity.

Specialists cannot mutate skills. Their existing permission-request flow asks
NyxBot to review a skill name/GUID with an optional version, or complete pins.
Specialists need no Ornn grant to request guidance; NyxBot resolves and previews
unknown pins with the managing person's visibility. Approval of that request is advisory:
NyxBot must apply it through `set_agent_skills`, which still requires the owner
card for external additions or re-pins. No permission decision alone attaches
external content. Owner UI writes are explicit owner actions.

Audit contains agent/skill IDs, versions, revisions and counts only. No package
content, credentials, request bodies or external descriptions enter audit or
Debug output.

## Rollout

No feature flag is needed: old replicas safely omit skill guidance and preserve
the sibling fields on grant updates. Deploy new replicas normally. There is no
new permission or execution boundary that an old replica can fail open.

## Interfaces

Human routes under `/api/v1/assistant/nyxagent` are `GET /skills/catalog`
(`q`, `page`, or `skill` GUID plus optional literal `version`) and
`GET/PUT /agents/{id}/skills`. PUT carries `{expected_revision, skills}`.
Metadata GET returns the immutable agent ID, revision, references and bounded
metadata. These routes inherit the assistant human-only and engine gates.
NyxBot uses `search_agent_skills`, `agent_skill_versions`, `preview_agent_skill`
and `get_agent_skills` before `set_agent_skills`. The setter requires the
immutable agent ID, not a mutable handle. Specialists use `request_agent_skills`
and their own `get_agent_skills`; all owner-turn agents have `skill_read`.
Ornn egress retains normal proxy policy, approvals, billing and node handling,
uses fixed read-only paths, and has a 30-second timeout per request.
Native reads carry the calling thread key as an API-key requester, including
live scoped-operation context; they never receive the browser-session approval
bypass. Human UI reads retain ordinary browser-session behavior.

## Owner-requested authoring

Reuse comes first: `search_agent_skills` → `preview_agent_skill` →
`set_agent_skills` with an owner card. Personal NyxBot gains `draft_agent_skill`
only under the existing default-off `assistant:agent-learning` flag. Authored
drafts share L1's encrypted review store and publisher; they do not enroll the
agent in automatic learning. Description is role/scope; persona is tone/style;
skills contain repeatable procedures, references, checklists and templates.

The card displays every text file and the generated publication frontmatter.
One approval packages server-side, privately publishes with the approving
person's signed identity and pins the verified exact version/hash. Revising an
owned private attached base uses PUT with the next version. No Playground,
sandbox, machine or model-selected raw upload is part of this path. Org
publication keeps L1's `owner_binding_unavailable` refusal, without a personal
fallback. See `AGENT_LEARNING.md` for bounds and fencing.

The card offers **Retry publication** for a conclusively
non-effective attempt, **Check again** for an uncertain or verified operation,
and **Request a new confirmation** for an expired card. A pending confirmation
offers **Deny**, which records the acknowledgement decision. An already decided
card offers **Discard** only while the operation is non-effective. A renewed
confirmation appears in the original conversation. Failure codes use fixed, sanitized
copy; Ornn response details and skill contents never enter audit. For an
update, NyxID snapshots the base interface before allocating the card so the
new frontmatter preserves category, output type, runtimes, dependency names,
environment variable names, tool names, and tags. If the base cannot be
reconstructed in Ornn's flat frontmatter, no card is allocated. Scripts and
other executable files from the base are not copied into the draft ZIP.

Owner-authored drafts allow ordinary URLs, example IDs and paths. Only credential
shapes are refused as private material, with safe rule/field/line diagnostics.
The 7,500-character total cap supports CJK and four-byte scripts through a
30,000-byte ceiling. Learned proposals retain their existing stricter checks.

## Adapter boundary

Ornn is the only skill registry, but its wire protocol stays in two modules.

- `services/agent_skill_service.rs` defines `OrnnReader` and classifies each
  request as `OrnnOutcome`: success, a local refusal or local error before
  dispatch, a complete response (status plus the problem-body root `code` mapped
  through a fixed `OrnnCode` table by `OrnnCode::from_problem_body`; the body
  read is bounded in size and time), a proxy error, an interrupted success
  body, or an uncertain timeout. Ordinary skill reads collapse it with
  `into_app_result`, which preserves the read error contract.
  `preview_checked` separates content that contradicts a pin (identity,
  dependency limits, hash, package) from a read that failed or came back
  malformed; only the former is an integrity failure for publication.
- `services/assistant_learning_publication.rs` owns frontmatter and interface
  snapshots, ZIP packaging, base verification, exact-version reconciliation and
  the mapping from registry codes to provider-neutral results: `PublishOutcome`
  (published, refused before mutation, version conflict, uncertain),
  `PublicationError`, `PublicationStage` and `FailureCode`.

The saga in `assistant_agent_learning_review.rs`, the card actions and the
frontend depend only on those neutral types and the non-effective predicate.
Generic proxy, acknowledgement, middleware and MCP code has no registry
branches. Failure codes that describe generic states are neutral
(`publish_uncertain`, `verify_failed`, `pin_conflict`, `target_busy`);
`ornn_*` codes mark refusals Ornn itself reported and remain stable strings.

Another registry adapter would implement:

1. An `OrnnReader`-equivalent reader that uses the acting person's identity and
   classifies outcomes without retaining response text.
2. Base verification (owner, private visibility, empty sharing, content hash,
   latest version) and an interface snapshot that its package format can
   reproduce exactly, or a refusal before any card exists.
3. Deterministic packaging with a content hash, and a publish call mapped to
   `PublishOutcome`, where only conclusive refusals count as non-mutating.
4. Exact-version reconciliation that verifies the downloaded bytes against the
   bound hash, and a mapping of its refusal codes to `FailureCode`.

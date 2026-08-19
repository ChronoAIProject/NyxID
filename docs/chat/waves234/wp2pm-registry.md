# WP-2PM — Wave 2 registry scaffolding + v9 bump (single-writer package)

**Master plan:** `docs/chat/waves234-plan.md` (§4, §5.1, §6). This brief doubles as the
**per-wave PM process definition** — WP-3PM and WP-4PM follow the same §Process with
their own tables.

**Mission.** Land everything the parallel Wave-2 teams (WP-2A, WP-2B) need in the
shared, conflict-prone files, so they never touch them: descriptors, Zod schemas,
registry entries, journey-id stubs. Then, at the deploy gate, land the v9 revision bump
as its own final PR. Also: file the Aevatar consumer issue for v9.

**Depends on:** WP-0A + WP-0B merged. **Blocks:** WP-2A, WP-2B (they build against your
stubs). The revision-bump PR additionally waits on the Aevatar v9 consumer deploy.

## Process (all waves)

1. **Scaffolding PR (merge early, no revision bump inside):**
   - `backend/src/handlers/assistant_actions.rs`: add the wave's descriptors (table
     below) to `MANIFEST_BODY`; extend the test-module `SUPPORTED_ACTIONS` with the
     wave's verbs; extend the golden-manifest test. **First wave only:** restructure the
     golden test to be table-driven (one `(action, risk, tier, remember, schema_fn)`
     row per verb) so later waves are ~1 line per verb; keep the full
     `assert_eq!(manifest, golden_manifest())` at the end.
     ⚠ Do NOT touch `ASSISTANT_ACTIONS_REVISION` in this PR. Descriptors published
     under the current revision are **silently skipped** by Aevatar's loader (verified
     — master plan §1.2), so shipping them early is safe.
   - `frontend/src/schemas/assistant-actions.ts`: one Zod params schema per verb
     (patterns: `requiredActionIdentitySchema` for ids, `requestedScopesSchema`-style
     deduped arrays, `.strict()` objects) + the `ActionCardParams` union variants
     (snake_case fields, `variant: "<verb_snake>"`).
   - `frontend/src/lib/assistant/action-registry.ts`: descriptor entries (title/body/
     cta copy per DESIGN.md voice — verify concrete hex/fonts against
     `frontend/src/app.css`, DESIGN.md is stale on those), `normalizeParams` branches
     (schema `safeParse` → variant, `{variant:"unknown"}` on failure), `ActionJourney`
     union ids.
   - `frontend/src/components/assistant/journeys/index.ts`: registry lines pointing at
     the module paths the teams will create. Unregistered/missing modules render the
     unsupported card (WP-0B guarantee), so this merges before the team packages exist.
   - `docs/chat/06-actions-registry.md`: extend the action table; record every place
     the browser Zod layer is stricter than the published schema (A6 rule).
2. **File the Aevatar consumer issue** (draft below) on `aevatarAI/aevatar`, owner
   eanz17. NyxID teams never edit the Aevatar repo.
3. **Teams run in parallel.** PM reviews their PRs; PM is the only merger of any edit
   that touches a §4.1 PM-owned file (there should be none).
4. **Revision-bump PR (last, deploy-gated):** flip `ASSISTANT_ACTIONS_REVISION` to the
   wave's revision, update the golden revision assertions and
   `docs/chat/06-actions-registry.md` revision strings (grep the whole tree for the old
   revision string — Wave 1's verified full consumer list was `assistant_actions.rs` +
   `06-actions-registry.md` only, re-grep anyway). Merge/deploy **only after** the
   Aevatar consumer for this revision is deployed and restart-verified against the live
   previous revision (master plan §6 order).
5. **Canary protocol** for the wave (per-family scripts live in team packages; PM runs
   them post-deploy and posts evidence on the wave issue).

## Wave-2 descriptor table (publish exactly; schemas are grammar-conformant and pin-ready)

Revision target: `nyxid-assistant-actions.v9` (assumes v8 = Wave 1 shipped; see master
plan §7 Q-A if not). Order: append the 15 after the existing 4, in this table's order.

Grammar reminder (enforced by both graders, fail-closed): closed objects
(`additionalProperties:false` + `properties`), arrays, strings **only** — no booleans,
integers, or enum keywords; no property name normalizing into the forbidden-secret set.

| # | action | params_schema (properties; all `type:string` unless noted) | required | risk | remember |
| --- | --- | --- | --- | --- | --- |
| 1 | `key.update` | `keyId`, `name`, `platform` | `keyId` | grant | false |
| 2 | `key.delete` | `keyId` | `keyId` | **destructive** | false |
| 3 | `key.extend_scope` | `keyId`, `addAllowedServiceIds` (array of string, minItems 1, maxItems 64, uniqueItems true) | both | grant | **false — never** (issue #1403 hard rule) |
| 4 | `key.bind_credential` | `keyId`, `serviceSlug`, `credentialLabel` | all | grant | **false — never** |
| 5 | `external_key.rotate` | `externalKeyId` | `externalKeyId` | grant | false |
| 6 | `external_key.delete` | `externalKeyId` | `externalKeyId` | **destructive** | false |
| 7 | `service.update` | `userServiceId` | `userServiceId` | grant | false |
| 8 | `service.delete` | `userServiceId` | `userServiceId` | **destructive** | false |
| 9 | `service.route` | `userServiceId`, `viaNodeId` | `userServiceId` | grant | false |
| 10 | `service.rotate_credential` | `userServiceId` | `userServiceId` | grant | false |
| 11 | `connection.revoke` | `serviceId` | `serviceId` | **destructive** | false |
| 12 | `provider.set_app_credentials` | `providerSlug` | `providerSlug` | grant | false |
| 13 | `provider.disconnect` | `providerSlug` | `providerSlug` | **destructive** | false |
| 14 | `endpoint.update` | `endpointId` | `endpointId` | grant | false |
| 15 | `endpoint.delete` | `endpointId` | `endpointId` | **destructive** | false |

Notes that are part of the contract:

- `provider.set_app_credentials` is the one Wave-2 verb with an **existing Aevatar
  parser** pinning exactly `{providerSlug}` (`EnsureOnlyProperties(root,
  "providerSlug")`, max 128) — publish exactly that, nothing more.
- `key.update` vs `key.extend_scope` split (master plan §7 Q-G, pending Calvin's
  confirmation): `key.update` carries only non-authority fields; the journey must
  refuse to widen `allowed_service_ids` / `allow_all_*`. Widening is exclusively
  `key.extend_scope`.
- Name checks done for this table: `credentialLabel` → `credentiallabel`,
  `externalKeyId` → `externalkeyid`, `addAllowedServiceIds` → `addallowedserviceids` —
  none are in the forbidden set. Re-run the conformance test locally anyway; it is the
  authority.
- Params carry the model's *intent* only; journey dialogs own every other editable
  field (rate limits, URLs, header names, …). Do not grow schemas to mirror the REST
  bodies.
- Descriptions: write in the shipped verbs' voice (see `SERVICE_REAUTHORIZE_DESCRIPTION`
  on the Wave-1 PR branch) — one "Use when …" sentence, one NyxID-owns-the-journey
  sentence, and the closing "Never ask the user for keys, tokens, or passwords in
  chat." For destructive verbs add "NyxID confirms this destructive action with the
  user every time."

## Aevatar consumer issue draft (file on aevatarAI/aevatar, reference #1403)

> **Consume NyxID assistant registry v9 (Wave 2 — key/service management).**
> Registry: `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs`.
> Required for every verb in [table]; per the Wave-1 lesson (NyxID
> `docs/chat/wave1-service-reauthorize-actions.md` §B), all sites below fail closed
> with the same symptom — `Load()` throws → `CreateDisabled()` → every NyxID action
> card dark on processes started after the NyxID deploy:
> 1. `protos/nyxid_chat_task.proto`: new `NyxIdAssistantActionKind` values for the 14
>    Wave-2 verbs without kinds (all except `provider.set_app_credentials`).
> 2. `SupportedActions` entries + `ParseX` parsers (params per NyxID's published v9
>    schemas, byte-exact).
> 3. `PinnedActionsByRevision["nyxid-assistant-actions.v9"]` = v8's four + all 15
>    Wave-2 verbs (pins are cumulative — a pinned action missing from the manifest is
>    `RegistryInvalid`).
> 4. `ExecutableActionsByRevision["…v9"]` = same set, only once (6) exists per verb.
> 5. **`IsActionExecutable` wire-action switch**: one arm per new Kind (B3 recurrence —
>    a missing arm silently falls to `null` and the verb stays dead).
> 6. Postcondition readers: `NyxIdActionPostconditionPort.VerifyAsync` arm + evidence
>    parser per verb, reading `GET /api/v1/assistant/evidence/{kind}/{id}` (new NyxID
>    surface, secret-free by construction — field lists in NyxID
>    `docs/chat/waves234/wp2a-*.md`/`wp2b-*.md`). Delete verbs: success evidence is a
>    clean 404 under valid auth.
> 7. **`ValidatePinnedContract`**: if v9 pins schemas, add v9 to any revision-keyed
>    special-case (B4 recurrence — today the `key.create` least-scope ternary).
> 8. `docs/contracts/nyxid-assistant-conformance/v1/registry-v9.json` = NyxID's exact
>    manifest; registry/postcondition/producer tests incl. failed/uncertain fixtures.
> Deploy order is binding: this issue deploys and restart-verifies against live v8
> **before** NyxID bumps.

## Acceptance criteria

- Scaffolding PR: `cargo test assistant_actions` green (19 descriptors, table-driven
  golden), frontend `test`/`lint`/`build` green, every Wave-2 card renders as
  supported-with-stub-journey (unsupported-card fallback) in the assistant test page.
- Bump PR: golden pins v9; deployed only per master-plan §6 order; post-deploy, an
  Aevatar restart loads v9 and Wave-2 verbs are executable end-to-end.
- Aevatar issue filed with the draft above + links to both team briefs.

## Test commands

```bash
source "$HOME/.cargo/env" 2>/dev/null
cargo test assistant_actions
npm --prefix frontend run test && npm --prefix frontend run lint && npm --prefix frontend run build
```

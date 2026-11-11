# BotFather rollout and live acceptance preparation

Primary-owned review artifact. Prepared locally; no command in this document
authorizes deployment, paid readiness calls, activation, or platform sends.
The completion gate is [the A1–A14 review record](2026-10-16-botfather-completion-review.md).

Current preparation uses the durable Heca recovery trees and the merged
`feat/native-platform-entry` branch. Historical `/tmp` artifacts referenced
below disappeared during the pause and are not available for replay. Unchanged
native2 was rebuilt and rechecked: formatting and all-target Clippy passed,
all targets passed 94/0/9, and the explicit public suite passed 18/0/0 with
three pinned Codex smokes. The rebuilt server retains the SHA shown below.

Local merged NyxID compilation has repeatedly reached its disk guard without
an executable. The next acceptance checkpoint is a draft PR with full
Rust 1.98.1 CI and a paired job in the private NyxAgent repository. That job
pins the public NyxID commit and runs its complete native suites plus explicit
legacy/native public pairs. No new native personal or paired-runtime pass is
claimed until those executions succeed. The longest-article FAQ draft is now
registered upstream and still requires its own checks. Deployment remains
unperformed.

## Release identity

Fill this table from the final reviewed source and produced artifacts before
requesting rollout authorization. Changing an entry after readiness requires a
new compatibility check and, when bound by deployment proof, a new probe.

| Component | Required recorded value | Current state |
| --- | --- | --- |
| NyxID backend | Commit and immutable deployment image digest | Pending complete implementation and backend execution |
| NyxID frontend | Commit and immutable asset/build identity | Pending cross-platform UI and final checks |
| NyxAgent API | Commit, executable SHA-256 and immutable deployment image digest | Reviewed base `6abecf8516da556e71ba4772af2e7beb5d695850` plus frozen native2 corrections independently verified; local server SHA `d71f74b8521c3aabe0ab90f6830431cba07ae13ad5ffe5266c15281967a34953`; final commit, actual NyxID pairing and deployment image pending |
| Codex runtime | Platform, release/version and executable SHA-256 | Local smoke uses full CLI 0.160.1 through a SHA-pinned app-server wrapper; deployment identity pending |
| Public policy | Exact JSON, selected catalog service/model/effort and policy digest | Four calls; 8,192 input and 512 output per call; 32,768/2,048 turn totals |
| Public funding | Sponsor ID and current selected pricing path | Preserve reviewed sponsor; 0.50 sender/day and 10 total/day |
| Public session storage | Shared encryption/binding configuration, conditional writes and object permissions | Deployment verification pending |

The local CLI SHA is
`09fa44fdc37a5fc70dc1ace31235f90468a2e193d0e85f7552eab068ea2582be`.
It is local synthetic-test evidence only. The production app-server must be
identified independently; copying this hash into production evidence is invalid.

Independent upstream checks on that exact clean commit passed: all targets
93/0/9 (passed/failed/ignored) and explicit public tests 21/0/0 including three
real Codex smokes. The subsequent frozen next3 corrections passed formatting,
whitespace, Clippy with warnings denied, all targets **94/0/9**, explicit public
tests **22/0/0**, and the server build. Its source hashes are recorded in
`/tmp/nyxagent-botfather-review-upstream-next3-source.json`; its local server
SHA-256 is `3666f7525977acb0d70ae65aaca3a4a28e7d64d15ec387d2f1c598a5f4039571`.
The
ordinary chat/research/channel cold-resume smoke separately passed 1/0 at the
preceding `f4da4471` checkpoint. Exact commands and local logs are retained in
the completion review. The new checkpoint preserves the exact legacy Telegram
wire and adds the explicit v2 platform identity and paired call-ID grammar.
Actual NyxID producer/adapter pairing and deployment acceptance remain pending;
these local results are not production release identities.

The actual frozen build21 pairing was explicitly executed and failed before
readiness or model work because its local JWKS fixture served an empty key.
The matching generated key/JWK correction and required fixture initializer are
registered in the build23 source freeze. Build22 was interrupted for the
initializer correction; build23 was interrupted by its owned disk guard.
Neither attempt produced a passing new executable. Upstream rejection was correct; neither this fixture correction nor
standalone upstream smoke results establish a successful paired journey.

The external native backend has passed copied metadata-only preflights through
metadata7. Metadata6 stopped on one test-helper return type; metadata7 contains
the correction and later journeys and passed with1,125 copied hashes independently
verified. These checks generate no
runnable test executable. A selected-test executable from a separately
attributed copy can supply narrow behavioral evidence, but release still
requires the complete registered checkpoint and compatibility replay. Native
journeys that seed profile/control state do not prove administrative conversion;
synthetic model output and counted loopback sends do not prove live receipt.

The selected3 test executable compiled successfully and was independently replayed:
65 passed,10 failed,0 ignored across16 focused suites with an explicit32MiB test
stack. Its source transform and binary identity are audited, but setup, delivery
and fixture failures still require corrected execution. Both legacy proof and
pre-existing saved-row/delivery compatibility cases pass in that selected copy.
See `/tmp/nyxid-botfather-review-selected3/results.json`. This does not authorize
native rollout or replace full registered backend acceptance. The separately
derived native tool-schema fixture preserves the raw capture. Native2 has now
passed actual Rust normalization equality, cold-resume/compaction smokes,
all-target94/0/9, explicit public22/0/0, formatting, Clippy with warnings denied
and the server build. The first native1 standalone server was rejected as stale
despite passing tests; a metadata-only own-crate rebuild produced the new server
identified above, with new description present and old description absent.
Aligned actual NyxID pairing remains pending. Evidence:
`/tmp/nyxagent-botfather-review-upstream-native2-artifact-audit.json`.

Native2's continuation and compaction measurements use the historical upstream
FAQ fixture and its first, unchanged 246-byte article. The staged neutral corpus
matches selected4, but its longest article is 1,020 bytes. Fixture alignment and
an actual longest-article continuation/compaction/cold-resume replay remain a
separate gate; copying the fixture alone does not establish that coverage.
Preserve native2 through the current actual pairing before making this change.

Selected4 was interrupted by its owned disk guard before producing a complete
object set or executable. Its frozen source and retained dependencies were
independently reverified; no selected4 tests or pairing ran. The incomplete
objects cannot be recovered with a relink alone. The compile/replay gate remains
open, with attribution in
`/tmp/nyxid-botfather-review-selected4-interrupted.json`.

The separately attributed jobs1 retry subsequently compiled successfully with
the same selected source/dependencies and an inherited one-slot codegen
jobserver. Primary verified executable SHA-256
`70870dbffbc53d8a28bfa8956619da90c157432102d5d425e37f60e0727b2343`
and all recorded inputs. Its focused behavior and actual pairing replay are
pending; full registered acceptance remains required. Evidence:
`/tmp/nyxid-botfather-review-selected4-jobs1-artifact.json`.

Primary independent jobs1 execution reproduced the worker's 74/1/0 across all
16 focused suites and 0/1/0 for the exact native pair. The remaining personal
failure occurs when verifying browser continuation's actual upstream request;
the terminal event name alone did not prove successful execution. The pair
passes discovery but fails at its first readiness probe, before Ready or runtime
activation. No full four-tool or complete native personal journey is established.
All frozen identities match and neither run timed out. Comparison/evidence:
`/tmp/nyxid-botfather-review-selected4-jobs1-replay-comparison.json`.

## Target and preparation

NyxID API origin: `https://nyx-api.chrono-ai.fun`.
Telegram entry: `@nyxid_manager_bot`, numeric bot ID `8631737144`.
Channel bot: `bb3f92a9-6cf9-405c-9bca-bc422c8555d5`.
Platform profile: `0b4949e8-be13-4abc-8b65-7b1d33d4aa11`.

Before release, record every serving replica's identities and ensure all are
compatible with the final wire/schema changes. Preserve an authority-enforcing
rollback build. Confirm the configured trusted NyxID origin and exact JWT issuer.
Public authority must not choose a JWKS origin or model callback destination.

NyxAgent storage needs conditional read/write access to `sessions/*`, `jobs/*`,
`public-sessions/*` and `public-revocations/*`, plus the necessary list permission.
Do not apply a lifecycle policy that deletes live revocation tombstones. Verify
cross-replica session resume and cleanup using the actual shared storage and
coordination services. Confirm the private loopback model bridge is not exposed.

Use the existing authenticated admin UI or CLI session; do not place credentials,
raw link bearers, confirmation codes, or chat content in this review document.

## Discovery, readiness and activation

The handlers in `backend/src/handlers/platform_bots.rs` define these operations:

| Step | Method and path | Required outcome |
| --- | --- | --- |
| Read profile | `GET /api/v1/admin/platform-bots/{profile_id}` | Current generation and active manager binding |
| Read runtime | `GET /api/v1/admin/platform-bots/{profile_id}/runtime` | Current audience settings, revisions, sponsor and policy |
| Discover | `POST /api/v1/admin/platform-bots/{profile_id}/public/profiles` | Selected public profile appears; discovery is not readiness |
| Probe | `POST /api/v1/admin/platform-bots/{profile_id}/public/readiness` | `public_readiness.status` is `ready`; bounded paid probe succeeds |
| Enable | `PUT /api/v1/admin/platform-bots/{profile_id}/runtime` | Reviewed public and personal audiences enabled with the exact current proof |
| Verify | Repeat runtime read and inspect deployed identities | Enabled policy digest and healthy runtime match approved artifacts |

Construct discovery and readiness bodies from a fresh profile/runtime read:

```json
{
  "initialize": "true only when runtime is null; encode as a boolean",
  "expected_profile_generation": "current profile generation; encode as an integer",
  "expected_manager_authority_epoch": "current epoch or zero for initialization; encode as an integer",
  "expected_personal_control_revision": "current revision or zero; encode as an integer",
  "expected_public_policy_revision": "current revision or zero; encode as an integer",
  "public_policy": {
    "source": "nyxagent",
    "profile": "selected discovered public profile",
    "sponsor_owner_user_id": "reviewed sponsor UUID",
    "total_daily_credits": "10",
    "sender_daily_credits": "0.5"
  }
}
```

This is a field worksheet, not a sendable payload. Produce a concrete validated
JSON file for final approval. Preserve existing explicit values if they differ
from initialization defaults, and resolve that difference in the approval result.
Do not reuse the generation-1 diagnostic snapshot as a current write precondition.

The enable body uses the same reviewed policy and revision fields, adds
`personal_enabled: true` and `public_enabled: true`, and sets
`public_deployment_digest` to the probe's **`public_readiness.policy_digest`**.
That field does not take `evidence_digest`. Do not clear an emergency stop or
replace the sponsor implicitly. Re-read and rebuild the proposal on a revision
conflict, expired proof, changed deployment, or changed pricing; do not retry an
old body blindly. Prepare separate explicit entry profiles for supported
non-Telegram transports after A8 implementation defines their final API.

## Native entry preparation

Before any non-Telegram activation, fill a separate reviewed record for each
entry with its platform, actual channel UUID and remote bot/account ID, direct
or gateway transport, transport generation, credential fingerprint and control
surface. For a gateway, include both local and upstream channel identities and
the bound version. Do not infer a shared entry from an ordinary channel route.
Record the completed native-identity migration and every serving replica's
compatibility before authoring v2 profiles. Preserve existing Telegram proof
and stored-origin compatibility in the released build.

Complete the replica upgrade before native setup writes. Older binaries can
recreate the retired global Telegram indexes and cannot read the new native
identity records; remove them from both serving and automatic restart paths.
After native records exist, rollback must use a reviewed build that understands
v2 and retains the shared-entry refusal markers. Keep the migration and command
ownership records intact; deleting them is not a rollback procedure.

| Platform | Selectable transport | Native input that must be observed | Setup and reply acceptance |
| --- | --- | --- | --- |
| Slack | Direct or gateway | Authenticated Events API private DM, bound to the selected app/team/bot user | Plain DM controls and FAQ/personal replies use the displayed control instructions; ordinary channel routing stays separate |
| Discord | Direct or gateway | Signed, fresh `/nyx` CHAT_INPUT interaction in the bot DM | Invalid URL-verification signatures return 401; signed PING succeeds; login/status/logout/confirm/ask and a personal action-card decision receive usable replies |
| Lark / Feishu | Direct | Authenticated/decrypted original p2p text event for the selected application | Displayed confirmation controls work through the actual adapter and reply destination |
| WhatsApp | Direct or gateway | Signed envelope filtered to the receiving phone-number ID | Original private text controls and replies retain the exact sender and destination |
| X | Direct | Signed Activity `dm.received` with exact receiving account, subscription tag and private participants | Existing subscription/billing policy is preserved; replies use the actual account and retain existing transport charging |

The capability response, setup UI and producer must agree on this matrix.
Unsupported gateway selections must refuse before marking a conversion pending,
refreshing a credential, charging verification or mutating a provider. Selecting
a supported gateway never authorizes direct delivery as a fallback.

Discord command installation is a separate external mutation in the final
approval payload. Record the authenticated application, verification-key match,
current `/nyx` observation, intended schema digest and durable installation
attempt. Show a create, owned-command update or exact recovery proposal as
appropriate. Preserve unrelated commands. If an earlier mutation has an uncertain
outcome, use the reviewed recovery path after its persisted dispatch deadline
and uncertainty window, with a fresh provider observation. Retiring a local
channel must not delete application-level command ownership or its history.

The release must also include the reviewed recovery/update lifecycle tracked by
R61 and R67. Exercise a pending setup after provider verification fails, a change
of configuring administrator or funding authority, an active direct entry after
credential replacement, and an active entry after its selected gateway retires.
Each recovery must show the current and proposed entry/transport identity and
relevant provider observation, preserve permanent ordinary-route separation,
invalidate old receipts/proofs/turns, and return the entry paused. Explicit
runtime enablement follows a fresh review and required readiness. The exact
API/UI contract and its executed evidence remain pending; removing a marker or
manually rewriting stored ownership is not an accepted repair path.

For both direct and gateway tests, prove that unavailable, retired, missing-control
and changed-credential entries consume traffic without ordinary-owner fallback.
Gateway acceptance must use an actual retained native receipt and the exact
content commitment; a generic human activity label, device event or normalized
message is insufficient. Check that shared-entry callbacks omit the operator's
relay access token. These are release acceptance requirements; the staged A8
helpers have not yet established them through execution.

## Live acceptance observation

After approval, use designated consenting test accounts. Record timestamps,
redacted receipt/turn IDs, observed result and deployed revision for each step.

Calculate sender/day headroom from the live selected rate before approving the
sequence. At the local funding fixture's 0.1069056 maximum per call and
0.4276224 maximum per loop, a 0.50 sender/day cap admits only one public turn:
even one forwarded call leaves less than the next full-loop reservation.
Actual wallet settlement does not refund conservative daily admission spend.
The before-login and after-logout FAQ observations therefore require a rate and
reviewed allowance that can fund both, or separate quota windows. Any temporary
test allowance must appear explicitly in the concrete rollout proposal with its
restoration step; do not change the existing configured cap implicitly.

1. A new Telegram sender asks a NyxID FAQ, obtains signup guidance, and sees
   logged-out status. Check approved FAQ/tool behavior and public sponsor usage.
2. Register under actual invite and email policy, sign in, and complete MFA when
   enabled. Preserve the original chat link through navigation. Review the exact
   NyxID account and confirm from the originating private Telegram chat.
3. Verify `/status`, personal tools/grants/mode, saved NyxID conversation,
   approval cards, and personal billing. Use a second account/sender to verify
   isolation and ensure the platform operator's account is never selected.
4. Exercise owned Telegram bot listing, setup selection, and native creation with
   the approved test resource. Confirm the shared manager remains platform-owned.
5. Start a new pending link, log out, and verify both pending proof cancellation
   and active personal authority revocation. Check that a late personal response
   is refused; ask a new public FAQ and verify logged-out status.
6. Repeat the public/login/personal/logout chat journey for one supported direct
   non-Telegram transport and one supported authenticated gateway transport.
   Verify opaque sender isolation, ordinary-channel compatibility and the absence
   of Telegram-specific management routing.
7. Continue a public conversation through a context refresh and process restart.
   Confirm the refresh notice follows persistence, the repeated question gets a
   complete normal turn, and public cost/call/continuation bounds remain enforced.

Local synthetic sends do not count as observed platform receipt. Capture actual
platform observations without recording secrets. Report any unsupported live
transport as incomplete acceptance, not as success inferred from local tests.

## Pause and rollback

Prepare a revision-fenced runtime update that disables both audiences using the
fresh current revisions. Apply it only as part of authorized rollout/rollback.
Verify refusal of new turns, cancellation/fencing of existing authority, and
durable cleanup before changing serving builds. Preserve public-session indexes,
revocation records and retirement markers throughout rollback.

An old binary that routes an opted-in or retired entry through ordinary owner
channels is not a rollback candidate. Restore the last reviewed compatible build,
rediscover and reprobe after any bound identity changes, and enable only after
the same acceptance checks. Budget spent by a real provider effect is not an
application rollback and must remain accurately settled.

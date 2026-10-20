# Independent audit witness (W8-3)

The reference executable is `nyxid-audit-witness`, built from the backend package.
It runs independently of MongoDB and of the NyxID application host. This batch
provides the protocol, client and sink. W8-4 adds the opt-in application relay and
local durable allocation, replica cutover readiness and intent/claim foundation.
Retained verification, retention, HMAC transitions and complete caller admission
remain tracked W8 integrations; installing this executable alone does not activate
them.

## W8-4 rollout decisions

PM condition C1 makes durable local audit unconditional and independent witness
deployment optional. With no witness configured or previously enrolled, startup,
authentication and ordinary work continue on the transactional persistent head
and outbox. Health, Integrity and verification must call this state `unwitnessed`;
local HMAC continuity is not external verification. Tests use the same local
mode unless they explicitly provision a witness. No test constructor creates a
fresh witness proof or overrides configured-witness admission.

The first adoption of an existing v1 chain requires draining every old writer,
setting `AUDIT_CUTOVER_DRAINED=true`, and declaring the complete identical
`SECURITY_ROLLOUT_REPLICAS` roster with `SECURITY_ROLLOUT_DRAINED=true` on every
replica. Each executing generation must publish a fresh compatible
`audit:durable-v1` observation; missing, duplicate, undeclared, undrained or
incompatible replicas block adoption. Startup continues publishing observations
while waiting for this initial roster to converge, before compatibility migrations
or business writers run. Initialization verifies the original chain and atomically
records its unchanged boundary. The audit acknowledgment and assessment are
consumed by this cutover, rather than required on each subsequent boot. A fresh
database needs no drain acknowledgment. Old binaries cannot safely be restarted
against the persistent head/outbox protocol. Their absence is an operator
assertion; observations cannot discover an unregistered old process.

Without a witness, `AUDIT_CHAIN_ID` may be omitted and the first initialization
persists a UUID v4. For later independent enrollment, retain and independently
provision that exact identity. A configured witness never automatically enrolls
or resets itself from MongoDB, and removing its configuration after enrollment
retains stale configured admission instead of reverting to optional unwitnessed
mode. Existing pending age/count can additionally make it backlogged.

PM approved C2's four-tier policy. Pending **age and count** gate DB-only
admission; contact freshness raises alarms and gates live-claim effects. The
former 30-second authentication-refusal proposal is superseded. The seven
positive limits below are configurable; soft limits must not exceed their hard
ceilings, Standard must not exceed Availability, and no age can exceed seven days.
No zero/disable value is accepted. A ceiling is reached at `>=`, so admission
cannot reserve the next event when its existing backlog is already at the ceiling.

| Tier | Fresh | Stale (any soft limit reached) | Own hard ceiling reached | Authenticated divergence |
| --- | --- | --- | --- | --- |
| Narrowing: revoke/disable/deny/logout-all | allow | allow + alarm | allow + alarm | allow + incident alarm |
| Availability: login/refresh/MFA enable/verify/recovery redemption/password change/polling/DB-only maintenance | allow | allow + alarm | refuse 13003 | allow + incident alarm, subject to the same hard ceiling |
| Standard: key create/rotate/grants/device approval/MFA disable/recovery regeneration/authority seeds/DB-only money movement | allow | allow + alarm | refuse 13003 | refuse + alarm 13004 |
| Effect: irreversible external dispatch | witnessed intent + live one-shot claim | defer, no dispatch | defer | refuse 13004, no claim |

Soft defaults are 60 seconds since authenticated contact, 30 seconds oldest
pending reservation, and 10,000 pending events. Standard's hard defaults are
3,600 seconds / 100,000 events; Availability's are 86,400 seconds / 1,000,000
events. Each worker inherits its actual operation's tier. Already admitted work
keeps reserved terminal-record capacity; completing it is not a new admission.
MFA disable is Standard despite its name, because it widens account access.
Narrowing is never refused for witness reasons; its ordinary database transaction
and security controls still apply. See the living inventory for mixed workflows.

An enrolled/configured process starting while the witness is unreachable starts
stale with unknown contact freshness; an older persisted backlog can additionally
be backlogged. It loads pending age from persisted `oldest_pending_at`, compared
with Mongo server time. Neither a restart, stored receipt, nor idempotent receipt
retry renews contact freshness. An authenticated challenge observation and
matching independent history are required. Quiet chains renew observations too.
A configured sink outage must not prevent process startup or drain every replica
from the load balancer. `/health` stays HTTP 200 and reports `degraded` plus an
`audit` object with witness state, per-tier decisions, counts, ages, sequences and
effective limits; unavailable local audit state is explicit. Integrity labels
local continuity separately and shows per-tier closure times. A quiet chain with
no pending reservations has no running pending-age deadline. Count limits may
close a tier sooner than its displayed age deadline.

13003/13004 return generic temporarily-unavailable 503 responses with
`Retry-After: 30`. Backlogged/witness-unavailable reasons remain operator-only;
these are admission controls, not proxy server faults. The runtime must record
categorical closure/divergence transitions as system audit events and emit
error-level transition/repeat logs at most once per minute, with recovery at info.
Logs contain fixed status/count/age/sequence metadata and never event bodies.
The policy/diagnostic foundation does not itself convert a workflow: each owner
must adopt transactional admission/reservations and consume the intent/claim
permit around its effect. Production activation requires the recovery gate below.
S3 compliance-mode Object Lock remains scheduled as W8-3b after W8-5, including
retained-version anti-shadowing and storage-level writer-denial tests.

The first W8-4 implementation group supplies local allocation and reservations:
`audit_chain_state` is the sole allocator for direct and caller-session appends;
the audit row, head advance and hash-only `audit_outbox` reservation share one
transaction. `audit_pending_events` accepts a typed principal/owner/stage,
UUID identities and a reviewed static action, with operation/stage uniqueness.
Its local materializer appends and marks the pending row in one transaction.
Neither materialization nor compaction releases unacknowledged capacity. Index
setup and compatibility migrations run after the allocator is initialized.
The original v1 canonical bytes and existing business transactions are retained.

The optional external relay authenticates independent history and supplies live
contact to configured admission. Categorical evidence is available in health and in the admin
verification/Integrity responses and UI. The audit and billing verify endpoints
use `status` for categorical evidence (`unwitnessed`, `unverified`, `unavailable`
or `broken`) and `local_status` for their existing local HMAC/anchor result.
Their `external_evidence` object and the same object on the rolling Integrity
response report local head, stored witness cursor, pending count and oldest
pending time independently of local continuity. A stored enrolled cursor is
`unverified` until a fresh process observation authenticates it. A missing head
is `unavailable`, including an empty local history. Older responses without an
evidence field display as unavailable; neither local success nor a missing field
produces a fully verified success message.
Replica/protocol observations gate first adoption only. Protected retained
verification and caller conversions remain explicit integration work. The local
head detects disagreement with a deleted tail, but a coordinated Mongo rollback
still requires independently authenticated witness evidence. Until W8-5 adds a
protected retention boundary, deleting the head row is corruption, including
an empty surviving tail; append cannot infer a replacement sequence.

## Authority and threat model

Provision a dedicated sink host, service UID and private group, with storage and
administrator credentials unavailable to the application host. The application
receives an append bearer credential, the fixed enrolled chain UUID and a pinned
Ed25519 public key through trusted deployment configuration outside MongoDB.
The application never receives the witness signing secret or enrollment access.
Its credential can append evidence, read protected evidence and request a fresh
signed observation. There are no reset, enrollment, deletion or rewrite routes.

Compromise of MongoDB **and the application host**, including historical audit
HMAC secrets and the append credential, cannot alter already protected sink
history, erase a dispatch claim, enroll the same chain again, or forge a receipt.
Changing an old audit hash or supplying only a larger sequence cannot replace
the recorded predecessor. Changing the application verifier/configuration on a
compromised host can make that host lie about its own status; independent readers
must retain the original public pin and chain UUID outside that host. HMAC keys
remain signing material, even when application policy calls them historical
verification keys. Full historical HMAC/retention acceptance belongs to W8-5.

A compromised append client can invent future authorized-looking evidence,
claim operations to deny service, or omit events. The witness cannot force a
malicious application to report an external effect, validate the truth of an
event, or prevent direct provider abuse. Hash links establish continuity; the
application must still verify each audit row's canonical bytes and HMAC before
submitting links. The witness does not receive HMAC secrets or event bodies.

Compromise of the sink process/service UID, signing key, sink administrator, OS,
filesystem or its backups is outside this reference sink's separation guarantee.
That principal can replace its writable journal or restore the entire sink.
Use independently protected backups/immutable storage and external receipt
retention if that threat must also be covered. Filesystem permissions protect
against the append principal, not a malicious sink administrator. Same-host
separate-UID tests prove the permission boundary, **not** resilience to host root.
Running both services on the application host, sharing its writable volumes or
administrative credentials, or mounting the sink's key/history there does not
qualify as production independence. No production infrastructure was provisioned
by this batch.

## Protocol version 1

The provider-neutral `WitnessClient` trait exposes `advance`, `observe`, bounded
`history` and `claim_dispatch`. `HttpWitness` implements the reference protocol;
a managed immutable-store adapter can implement the same authenticated contract.
Adapters can create a process-local `ObservationChallenge`, send its read-only
request and consume it to authenticate the signed response into `VerifiedContact`.
The challenge cannot be cloned/deserialized or supplied with an old nonce/clock.
The application runtime must validate provider independence and enrollment at
activation; the trait alone cannot prove deployment isolation.

Every receipt signs `protocol`, `chain_id`, `receipt_seq`, `previous_receipt`,
`witness_time_ms`, the current audit head (`key_version`, `seq`, `prev_hash`,
`entry_hash`), and the complete typed event:

| Event | Contract |
| --- | --- |
| `enrollment` | Offline admin only; authenticates the independently reviewed existing v1 tail, or explicit sequence-zero genesis with zero hashes. The chain UUID is UUID v4. Creates receipt zero exactly once. |
| `advance` | Application supplies UUID-v4 request identity, the exact prior receipt digest and 1–128 contiguous audit links. Every link must be the preceding sequence plus one and name its exact hash. Gaps, non-monotonic sequences, forks, a different chain and arbitrary key-version changes fail. |
| `dispatch` | Binds chain, logical operation, attempt, process, exact witnessed intent sequence/hash and prior receipt digest. One claim per logical operation persists independently of MongoDB. No lease expiration or reassignment exists. |

An identical accepted request returns its original receipt, including its original
time, even after restart or later appends. A changed request with the same identity
fails. A caller with a stale predecessor must first authenticate current history;
it must never reset or silently accept a changed local prefix. All receipts remain
available. Key versions are positive integers; this batch permits only the enrolled
version. A later authenticated transition protocol must explicitly enable a new
version. There is no arbitrary hash replacement or key-switch escape hatch.

`POST /v1/advance`, `/v1/dispatch`, `/v1/history`, and `/v1/observe` require exactly
one `Authorization: Bearer ...` line. The provisioned token is 32 random bytes
encoded as lowercase hex; only its SHA-256 digest is in enrollment. Comparisons
use constant-time digest equality. Requests and responses are bounded to 256 KiB;
history is bounded by both 128 receipts and response bytes. Continue from the last
returned receipt sequence plus one, bind the page's first predecessor to the
previous authenticated receipt, and compare the final cursor to a fresh observation.
An empty page alone proves no current head. Unknown fields are rejected.

Observation uses a newly generated random 32-byte challenge for **each** call.
The signed answer binds that challenge, chain, witness-side current time and
latest receipt digest. The HTTP client verifies the public pin, challenge and
30-second clock-skew allowance, and refuses a wall-clock jump during the exchange.
`VerifiedContact` records the start of contact with process-monotonic `Instant`;
its age includes transport time. It cannot be reconstructed by deserializing a
persisted receipt. A quiet chain can renew contact without adding audit rows.
An old signed receipt or old observation cannot renew freshness. Unconfigured,
unreachable, unauthenticated, replayed or divergent evidence is an error, never a
verified-success result. Admission's freshness/lag thresholds arrive in W8-4.

Dispatch claim retrieval is evidence, **not** a reusable dispatch authorization.
The future effect runtime must consume its own process/attempt claim once in
memory. A new process finding an accepted claim without a durable terminal
outcome must treat it as uncertain. A paused old worker retains any provider-side
ability it already had; a local lease cannot revoke that ability. Another send
requires real provider idempotency or positive provider reconciliation, with the
same logical operation identity. W8-3 tests claim persistence and non-reassignment;
real restored-Mongo and paused-provider caller tests remain pending integration.

### Canonical signature bytes

This encoding is separate from, and changes none of, the existing audit/billing
v1 canonical bytes. Integers, including version and variant tags, are unsigned
64-bit big endian. `field(bytes)` is a u64 byte length followed by the bytes.
UUIDs use their 16 raw RFC4122 bytes; hashes and signatures use lowercase hex ASCII.
No delimiter concatenation, optional free-form metadata or JSON canonicalization
is involved.

Receipt order is `field("nyxid.audit-witness.receipt")`, protocol,
`field(chain_uuid)`, receipt sequence, `field(previous_receipt)`, witness time,
head link, then the event. A link is key version, sequence, `field(prev_hash)`,
`field(entry_hash)`. Event zero contains the enrollment link; event one contains
`field(chain_uuid)`, `field(request_uuid)`, `field(previous_receipt)`, link count
and every link; event two contains `field(chain_uuid)`, `field(operation_uuid)`,
`field(attempt_uuid)`, `field(process_uuid)`, intent sequence, `field(intent_hash)`
and `field(previous_receipt)`. The signature is strict Ed25519 over these bytes.
The receipt digest is SHA-256 of those bytes followed by `field(signature_hex)`.

Observation bytes are `field("nyxid.audit-witness.observation")`, protocol,
`field(chain_uuid)`, `field(challenge_hex)`, witness time and
`field(latest_receipt_digest)`, signed with the independent key. Domain separation
prevents a receipt signature from being an observation signature. The permission
harness independently implements receipt framing in Python for its live app-UID
requests. Body/property order and JSON whitespace do not affect signatures.

## Storage, durability and enrollment

### Intent and outcome reservation

The service-level foundation accepts only reviewed static action names and typed
actor/owner/target identities. A business owner creates one stable operation UUID
outside retries, then commits its DB mutation, the intent event and **two** pending
slots in the same transaction: one for intent, one for the eventual outcome.
The outcome slot has no event body before completion. Witnessing the intent
releases only its own slot. Outstanding outcome reservations remain in pending
count/age admission, including while no materialized event occupies that slot.
Terminal success/failure/cancellation/uncertainty consumes the original slot,
so closed admission cannot prevent recording an already admitted result.

With a configured/enrolled witness, dispatch waits for the materialized intent
to appear in authenticated observed history. The application records the attempt
and serving generation **before** requesting the independent claim. It obtains
an exact operation/attempt/process/intent-bound receipt and records the claim
before issuing a non-cloneable, non-serializable permit. Consuming that permit
rechecks its durable phase/identity, fresh Effect admission and any shared
divergence marker before entering the provider closure. The same generation
UUID is used by the deployment's existing replica identity. Claim retrieval and
stored receipts do not renew contact freshness. Failure after a potentially
accepted claim leaves its reservation and attempted identity intact.

A recovered unresolved attempt is durably `uncertain` and yields no send permit.
No lease timeout, missing local response, new process or retrieved receipt grants
reassignment. A provider call already entered by a paused original process may
still finish; the new process must not dispatch again. Positive provider
reconciliation or actual provider idempotency belongs to each owning workflow
and remains W8-6 work. Marking uncertainty does not claim that no effect happened.
There is no generic retry/reset endpoint. The process remembers issued logical
operations for its lifetime; capacity planning must include that memory.

Witness-free local mode retains the same intent/outcome transaction and local
one-shot claim, while permitting normal operation under C1. It remains explicitly
`unwitnessed` and provides no independent protection against a rollback of local
claim history. A deployment that was enrolled cannot enter this mode by removing
its witness settings. These are infrastructure APIs; every pending workflow must
still adopt stable business dedupe identities, terminal recording and reviewed
provider reconciliation before it counts as converted.

### Application pin and relay

Set `AUDIT_WITNESS_URL`, `AUDIT_WITNESS_PIN_FILE` and
`AUDIT_WITNESS_TOKEN_FILE` together. Partial configuration fails startup. The
public pin is JSON with `protocol: 1`, the lowercase hex `verification_key`, and
`enrollment`, the original complete signed receipt zero obtained and verified by
the independent administrator. Its enrollment event must equal its head. Keep
the original pin and receipt outside rollbackable Mongo and outside app-host
custody. The app copy is an administrator-owned read-only file whose directory
ancestors are neither application-owned nor writable by group/world; all path
components are opened without following symlinks. Run the app under a distinct
unprivileged UID. On macOS use canonical absolute paths. The append credential
is a separate app-owned mode `0600` file with no additional hard links, symlinks
or group/world-writable ancestors. Neither raw credential nor verification pin
appears in diagnostics or Debug. The library's loopback validation constructor
is not exposed as a production configuration downgrade.

No network call blocks startup. A configured process starts `stale`, with unknown
contact freshness and Mongo's existing oldest-pending age. Validating the protected
enrollment pin durably activates configured admission before serving, even if the
first observation is unreachable; removing settings cannot undo that activation.
The activation does not advance the witnessed cursor or renew contact. Every five seconds,
including on an otherwise quiet chain, the relay challenges the witness and
reconstructs authenticated history from the protected enrollment. Pages must
bind the previous signed receipt, preserve monotonic receipt order/time, and
agree with the final challenge-bound observation. Every newly traversed audit
link is compared with its local canonical HMAC row. Each process keeps only live
contact in memory; no persisted receipt can create it. Full history replay is
bounded per tick, continues from the authenticated process cursor, and publishes
no freshness before catching up. Receipt digest memory/history replay cost must
be capacity-planned until protected W8-5 checkpoints provide a retained boundary.

The relay commits a UUID-identified advance request, its exact predecessor and
bounded links before sending. Transaction conflicts fence competing workers;
the witness serializes requests at the exact predecessor. An identical retry
keeps its original identity. An accepted request whose response or Mongo commit
was lost is recovered from signed history, without manufacturing a second batch.
Only authenticated observed history releases capacity: marking outbox rows,
advancing the witnessed cursor, clearing a completed request, decrementing the
pending count and recalculating the remaining oldest reservation share one
majority+journal Mongo transaction. Local materialization never releases it.
Pending requests are rechecked against real canonical rows before any retry.

A sibling dispatch claim can consume an advance request's predecessor without
moving the audit head. After authenticating that successor history, the relay
can rebuild an unaccepted advance from the unchanged canonical rows at the new
receipt cursor: its old predecessor is spent, and its links extend beyond the
authenticated head, so the old request cannot have been accepted or be accepted
later. While its predecessor remains current, retries retain the original request
identity. This advance-only rescheduling never reassigns an effect claim or
authorizes another provider send.

Conflicting enrollment/history/local rows latch a durable incident and remove
live contact. Startup reloads that incident even while the witness is unreachable.
There is no application method to clear it. Removing configuration after
enrollment preserves stale configured admission, rather than reverting to
unwitnessed policy. Protection against a rollback of both the incident marker and
all local state depends on independently retained witness history and the W8-5
recovery/checkpoint protocol; do not treat Mongo's incident flag as external
evidence. A current challenge and agreement support live diagnostics; the verify
API continues reporting `unverified` until complete retained external verification
lands. Configuring this relay does not satisfy the production recovery gate or
convert pending workflows.

The executable requires Unix permissions and an unprivileged sink UID distinct
from the admin owner. Its enrollment directory is admin-owned mode `0750`;
`enrollment.json` and `signing.key` are admin-owned mode `0440`, held by the sink's
dedicated effective group. The journal directory is sink-owned `0700`, lock
`0600`, and published records `0400`. No other application identity may belong
to that group. Startup checks owners, exact modes, group custody, symlinks and
writable ancestors, and refuses unsafe permissions without repairing them.
Root-owned sticky temporary parents are allowed for isolated validation.
Paths must be absolute, without symlink ancestors (on macOS use canonical
`/private/tmp`, not the `/tmp` symlink). Neither root-run service nor same-UID
enrollment/service custody is accepted. An exclusive OS file lock prevents two
sink processes from appending to one journal.

Enrollment is an offline root/admin command. All three destinations must be new;
interrupted provisioning is an error requiring independent administrator inspection.
There is deliberately no force/re-enroll/reset option. It creates a random signing
seed and token, fsyncs files, ownership/permission changes and containing directories,
and commits the signed enrollment record. The raw token output is mode `0600`;
deliver it securely to the app principal. Do not copy the signing seed to the app.

Each journal record uses this publication sequence:

1. Create a new private `.pending-<uuid>` file with exclusive creation and no
   symlink following; write the complete signed JSON receipt.
2. Fsync that file. A record is at most 128 KiB.
3. Hard-link it to its zero-padded receipt-sequence filename. `link(2)` refuses
   any existing target; a rename that overwrites is never used.
4. Fsync the journal directory, remove the unpublished temporary name, then
   fsync the directory again. Only then update memory and acknowledge.

The service never rewrites, truncates or deletes a published record. I/O failure
poisons the running writer: even observations fail until restart, because a
record may already have been published. Recovery sorts sequence filenames,
requires a contiguous receipt history from independently enrolled receipt zero,
and validates every signature, predecessor, timestamp ordering, audit link and
dispatch identity. Malformed/modified complete records and missing middle records
fail closed. Unpublished temporary files are ignored and retained for admin
inspection; a crash after publication can leave its second hard link. No tail
truncation or automatic repair occurs.

Use a local filesystem that implements hard links, exclusive locks, atomic link
publication and durable file/directory fsync. Do not substitute a filesystem with
weaker semantics or a shared writable network mount without separate validation.
Storage growth, full-history replay time and capacity are operator responsibilities
for this reference sink; no automatic compaction/pruning exists. Keep disk and
host alerts independent of the app. The process-crash tests do not emulate hardware
power loss, controller-cache loss or a dishonest fsync implementation.

## Build and provision

```sh
CARGO_BUILD_JOBS=2 cargo build -p nyxid --bin nyxid-audit-witness --features aws-kms,gcp-kms
```

The executable uses flags and protected files, with **no new production environment
variables** in this batch. First provision the independent host, protected parent
directories, dedicated service UID/GID, TLS certificate/ingress, monitoring and
backups. Provisioning is a deployment prerequisite. Example after assigning a
dedicated UID/GID 900 and independently authenticating the existing audit tail:

```sh
sudo nyxid-audit-witness enroll \
  --admin-dir /etc/nyxid-witness \
  --journal-dir /var/lib/nyxid-witness \
  --token-output /root/nyxid-witness-append.credential \
  --sink-uid 900 --sink-gid 900 \
  --chain-id 6d53db16-51f6-491c-ad87-39f27e413e75 \
  --key-version 1 --seq 0 \
  --prev-hash 0000000000000000000000000000000000000000000000000000000000000000 \
  --entry-hash 0000000000000000000000000000000000000000000000000000000000000000
```

The example is a **new-chain genesis** only; never apply it to existing audit
history. Generate a deployment-specific chain UUID, authenticate any adopted v1
tail and pin the resulting public key outside Mongo and the application host.
Start `serve` as that dedicated sink UID/GID, with cleared supplementary groups:

```sh
nyxid-audit-witness serve --admin-dir /etc/nyxid-witness \
  --journal-dir /var/lib/nyxid-witness --listen 127.0.0.1:8091
```

The listener refuses non-loopback binds. Put a TLS reverse proxy on the sink host
in front of it; expose only these four endpoints to the app, preserve the single
Authorization line, bound bodies and disable request-body/token logging and
response caching. The production client accepts HTTPS origins, refuses embedded
credentials, queries/fragments/path prefixes and redirects, and does not use
environment-configured HTTP proxies. Its explicitly named loopback validation
constructor permits literal loopback HTTP for tests and never proves production
independence. Do not present plaintext loopback validation as remote TLS testing.

## Executed acceptance and remaining integration

The backend library tests exercise signatures, framing, every link, fork/gap/
sequence/chain/key-version refusal, conditional/idempotent replay, contention,
adoption, fresh challenges, transport failure, bounded history, unsafe storage
refusal, corrupt/missing middle records and independent dispatch non-reassignment.
Four real child-process aborts cover write-before-fsync, file-fsync-before-publish,
publish-before-directory-fsync and directory-fsync-before-ack. Restart resumes
from complete published history and retry produces one logical receipt.

`backend/tests/audit_witness_permissions.py` runs the **real executable** in a
disposable Linux guest under numeric UID/GID 65533 for the sink and 65534 for the
append client, with no supplementary groups. The client actually authenticates
and appends, then kernel permissions deny read/rewrite/truncate/unlink/rename/
replacement/chmod/hard-link of history, signing-key read, and enrollment changes.
The sink UID also cannot modify admin enrollment. Both application and sink UIDs
cannot invoke offline enrollment; an admin cannot overwrite existing enrollment.
The harness sends SIGKILL after acknowledged writes, restarts, and checks the
same receipt. Strace checks successful file-fsync → link → directory-fsync order,
tracing no read/write buffers. Run it as root only inside an isolated validation
guest; it creates/deletes only its own `/tmp/nyxid-w8-witness-permissions-*` fixture.
It fails rather than skips if prerequisites are missing. Batch command/result
counts are in [SECURITY_W8_LOG.md](SECURITY_W8_LOG.md).

W8-4 adds application configuration, relay, local-row comparisons, AppErrors,
outbox/lag diagnostics and one-shot intent/claim consumption. Its signed-provider
tests exercise restored Mongo, response loss and paused/restarted dispatch at
the foundation boundary. Pending: protected retained verification/checkpoints
and key transitions, independent production
provisioning, the Object Lock provider, scoped audit UI, and every remaining
critical workflow's adoption and provider-specific reconciliation/crash tests.
Keep these pending in the living inventory.

## Admin-custody replacement and DR restore

**Production rollout gate:** do not enable configured-witness admission until an
independent administrator owns this runbook, has inventoried the chain UUID,
original enrollment public pin, protected receipt/checkpoint history and retention
policy outside the application/Mongo hosts, and has rehearsed replacement and DR
restore in an isolated replica-set environment. W8-5's protected recovery
checkpoint/epoch protocol and its tests must be accepted before this gate can
pass. The current reference sink has no recovery/checkpoint mutation interface;
the steps requiring that protocol are prerequisites, not executable recovery
features of the W8-3 sink. A document acknowledgment alone does not authorize an
arbitrary re-sign, reset, fresh genesis or new pin. Keep normal witness-free local
development available under C1.

There is **no application endpoint to clear divergence**. Application credentials
cannot enroll/re-enroll, choose a new pin, delete prior evidence, or approve a
recovery checkpoint. Operators must not change Mongo's `witness_enrolled` flag,
remove witness configuration, rewrite the local head, truncate the outbox, raise
ceilings to bypass an incident, or restore the witness from the same rollbackable
backup as Mongo. Preserve the incident evidence even if the application/HMAC
signing material is compromised.

1. **Contain and preserve.** Stop new Standard/Effect work through the existing
   admission fence. Keep Narrowing and bounded Availability available for
   investigation. Capture independent signed observations/history, protected
   checkpoints and dispatch claims under administrator custody; retain the local
   database and diagnostics without exporting bodies or secrets. If effects are
   in flight, classify them uncertain and reconcile with the provider using their
   stable logical identity. A fresh process never reuses a retrieved claim to send.
2. **Establish the last trusted boundary.** From a separate verification host,
   retain the original chain UUID/pin and authenticate every receipt from the
   independent enrollment/checkpoint through the latest obtainable observation.
   Compare local canonical HMAC links, sequence/hash and retained billing anchors
   to that history. Historical HMAC keys can sign and cannot independently prove
   that a replacement local tail is authentic. If any externally recorded segment
   is missing, recover it or document the unresolved gap; do not invent events.
3. **Destroyed witness, intact authenticated local history.** Provision a new
   independent protected sink with fresh administrator-owned custody and signing
   pin. Authenticate the proposed local boundary against surviving independent
   receipts/checkpoints plus the incident investigation; an unwitnessed suffix
   requires explicit independent review and remains a disclosed evidence gap.
   Under W8-5's approved recovery protocol, create an administrator-authorized
   replacement checkpoint/epoch binding the original identity and pin, last
   protected receipt, reviewed local head, new pin and reason. Preserve both
   historical verification paths; never replace original history with newly signed
   versions. Distribute the approved enrollment/epoch and public trust change
   outside Mongo to every drained replica. The app append credential cannot do it.
4. **Legitimate Mongo DR restore behind the witness.** Retain the witness and its
   monotonic history. Restore the missing local suffix from protected recovery
   sources and prove the local head/claims match it. If suffix recovery is
   impossible, remain diverged until a W8-5 administrator-approved DR checkpoint
   explicitly records the rollback boundary, lost interval and protected old
   history in a new recovery epoch. Restored dispatched work remains uncertain;
   leases and missing Mongo outcomes never release protected operation claims.
   Reconcile billing balances/anchors before admitting new money movement.
5. **Resume and verify.** With old writers drained, verify the authorized checkpoint
   and retained key versions, obtain a new challenge-bound observation, reconcile
   pending reservations without dropping them, and verify each tier's diagnostics
   and time to closure. Re-enable Standard/Effect only after fresh authenticated
   agreement and a successful one-shot claim rehearsal. Keep the incident's
   system audit transitions and administrator recovery authorization permanently
   verifiable under the applicable retention policy.

If no independent trusted boundary or valid recovery authorization survives,
there is no safe automatic recovery path. Continue investigation with protected
Narrowing/Availability access; do not report the recovered deployment as verified.

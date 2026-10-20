# Independent audit witness (W8-3)

The reference executable is `nyxid-audit-witness`, built from the backend package.
It runs independently of MongoDB and of the NyxID application host. This batch
provides the protocol, client and sink. Application readiness, outbox delivery,
verification, retention, HMAC transitions and caller admission are subsequent W8
integrations; installing this executable alone does not activate those controls.

## W8-4 rollout decisions

PM condition C1 makes durable local audit unconditional and independent witness
deployment optional. With no witness configured or previously enrolled, startup,
authentication and ordinary work continue on the transactional persistent head
and outbox. Health, Integrity and verification must call this state `unwitnessed`;
local HMAC continuity is not external verification. Tests use the same local
mode unless they explicitly provision a witness. No test constructor creates a
fresh witness proof or overrides configured-witness admission.

The first adoption of an existing v1 chain requires draining every old writer
and setting `AUDIT_CUTOVER_DRAINED=true`. Initialization verifies the original
chain and atomically records its unchanged boundary. The acknowledgment is
consumed by this cutover, rather than required on each subsequent boot. A fresh
database needs no drain acknowledgment. Old binaries cannot safely be restarted
against the persistent head/outbox protocol. Their absence is an operator
assertion; observations cannot discover an unregistered old process.

Without a witness, `AUDIT_CHAIN_ID` may be omitted and the first initialization
persists a UUID v4. For later independent enrollment, retain and independently
provision that exact identity. A configured witness never automatically enrolls
or resets itself from MongoDB, and removing its configuration after enrollment
must report unavailable instead of reverting to optional unwitnessed mode.

PM condition C2 requires separate approval before coding configured-witness
admission. Proposed defaults are 60 seconds fresh contact, 30 seconds oldest
unacknowledged reservation and 10,000 pending events. The pending decision table
is recorded in the living inventory; no policy implementation is implied by
the head/outbox foundation. S3 compliance-mode Object Lock is scheduled as
W8-3b after W8-5, including retained-version anti-shadowing and storage-level
writer-denial tests.

The first W8-4 implementation group supplies local allocation and reservations:
`audit_chain_state` is the sole allocator for direct and caller-session appends;
the audit row, head advance and hash-only `audit_outbox` reservation share one
transaction. `audit_pending_events` accepts a typed principal/owner/stage,
UUID identities and a reviewed static action, with operation/stage uniqueness.
Its local materializer appends and marks the pending row in one transaction.
Neither materialization nor compaction releases unacknowledged capacity. Index
setup and compatibility migrations run after the allocator is initialized.
The original v1 canonical bytes and existing business transactions are retained.

This group does not yet connect the external relay or implement configured
witness admission. Categorical evidence is available in local verification and
health; the admin Integrity DTO/UI must also carry it before W8-4 is complete.
The immutable external cursor, replica/protocol observations, protected intent
dispatch and caller conversions remain explicit integration work. The local
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

Pending: integration of the client with the server's reserved AppError block,
independent deployment/readiness and writer drain, Mongo outbox/lag counters,
live local-row verification against the witness, retention/checkpoint and
key-version transitions, provider dispatch one-shot consumption/reconciliation,
full restored-Mongo/paused-provider tests, scoped audit UI, and the remaining
critical workflow conversions. Keep these pending in the living inventory.

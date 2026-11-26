# Crash and error reporting

This is the normative contract for issue #2281. The `diagnostics:crash-reports`
feature flag and matching fleet readiness capability default off. NyxID stores
sanitized reports itself and provides `/admin/crash-reports` to first-party
human administrators and operators. Sentry export is optional and server-only.
Counts are **reported events**, not a crash-free percentage, unique people or
complete incident coverage.

## Coverage and consent

Web automatically observes window errors, same-origin app script/stylesheet/modulepreload failures, unhandled promise
rejections, React root callbacks and router render failures. Existing browser
error handling, React fallback rendering and chunk-recovery reloads continue.
Cancellation, handled asset recovery and API refusals below HTTP 500 are excluded.
Web keeps no persistent report buffer and makes no unload/beacon submissions.
It permits at most ten reports per page, four concurrent admissions, and dedupes
error objects across hooks plus equivalent reports for sixty seconds.

iOS captures JavaScript errors/fatals and native signal crashes using a local
Expo module and PLCrashReporter. An uncaught NSException can terminate as a signal;
its exception name and reason are never read by the sanitizer. Android reports
**JavaScript errors and JavaScript fatal errors only**. Android native JVM/NDK
crashes are **not captured**. No Android native crash handler is installed.
The CLI and backend process crashes are outside this feature's capture coverage.

Diagnostics requires an authenticated first-party human, live account analytics
permission acknowledged at policy `2`, and an account-bound device choice at
policy 2 or newer. Browser DNT and GPC deny capture. Sensitive web routes use the
existing analytics restrictions. Every send fetches permission afresh, checks
account/session/device generations, then submits the exact revision. The server
checks that revision on admission and again immediately before each export.
An already dispatched vendor request cannot be recalled.

Withdrawing permission stops new reports and pending exports; reports NyxID has already received remain for up to 30 days unless the account is deleted, and exported vendor copies expire under vendor retention.

Policy-1 users keep categorical analytics. Legacy stored device choices mean
policy 1; a PUT without `policy_version` still records `1`. New clients negotiate
`current_policy_version` before sending `2`. The additive `diagnostics_offered`
field requires serving readiness, the acting person’s flag and no global kill
switch; only then is current policy 2 advertised and diagnostic UI displayed.
The entire display-status computation is bounded to 250 ms and fails closed.
`diagnostics_status` distinguishes `available`, `consent_required`, definitive
`not_offered` and `unknown`. Timeout, lookup errors and unserving readiness are
unknown; old responses without a status also mean unknown to current clients.
Unknown pauses capture and preserves pending records. Only definite withdrawal,
account change or `not_offered` purges them. The default-off path caches only
absence of enabled flag scopes, for at most five seconds; it never caches a
positive decision.

`GET /api/v1/public/config` exposes the optional discovery hint
`crash_diagnostics_offered: true` when the rollout is Global or Cohort. It omits
the field for Off or Killed, preserving the default-off response shape. The hint
starts false and the public handler reads only process memory for this field,
without database commands or person-specific state. A tracked, drain-aware worker
evaluates `admin::rollout` at startup and every 60 seconds, separately from the
unchanged 15-minute manifest refresh. Local admin flag writes recompute the hint
through rollout-cache invalidation before returning, with a 200 ms read bound;
other replicas converge on their next successful refresh within the cadence.
Failed reads retain the last value. This hint only permits the banner's cached
preference GET; it grants no offer or consent authority. Only this hint enables
the banner's preference read, at most once per session, and explicit saves update
that cache. Without the hint, product telemetry retains its first-time disclosure
without reading preferences. Mobile reuses the hint from its existing domain
bootstrap or sign-in public-config response, without an extra discovery request.
Builds without a second domain set (`EXPO_PUBLIC_NEW_*` unset, for example dev)
make no launch-time public-config request, as on main. Their hint arrives only
from an in-app sign-in's existing public-config response, so their review notice
appears only after a sign-in. Production builds with the new domain set refresh
the hint every launch. Settings › Privacy always offers review when diagnostics
are offered and updated permission is needed, independently of the launch hint.
A missing hint suppresses the mobile review notice. A Cohort non-member on a
crash-only deployment sees no banner because their preference response does not
offer diagnostics.

Web includes diagnostics in the first consent footer when offered. Accounts
that already answered are not prompted again; Settings › Privacy provides
the updated permission review dialog. Mobile offers an account-bound one-time
Review / Not now alert and a Privacy review row. Not now retains the old choice.
Local withdrawal remains usable during network failures.

### Default-off footprint

With rollout Off or Killed, no DSN, an absent public hint and no previous policy-2
device opt-in, clients add no requests beyond main's: the web banner and mobile
notice skip preference discovery. The export worker issues no database
commands, and the manifest worker makes no manifest fetch or registry write.
The remaining work is:

- Startup index creation for `client_error_reports`,
  `client_error_report_counters` and `client_release_registry`, plus the
  `crash_enabled_scope` index on the existing `feature_flag_overrides` collection.
- Per replica, one indexed `feature_flag_overrides` find at startup and every
  60 seconds for the public hint, and at startup and every 15 minutes for the
  manifest gate. A large override inventory can require cursor `getMore` commands.
  The public handler itself reads the hint from process memory.
- On main's existing analytics preference GET/PUT requests, the
  `has_enabled_scope` absence check. Only absence is cached, for at most five
  seconds per replica; a warm absence cache adds no database command.

The limits matter after an earlier rollout: mobile's capture lifecycle still
reads preferences for a hydrated, enabled policy-2 device on session restoration,
consent changes and foregrounding, independently of the notice hint. A retained
true hint can also permit discovery reads until rollout refresh succeeds. If a
global kill leaves narrower enabled overrides, the absence cache cannot apply;
preference requests still resolve memberships and the flag overrides before
returning `not_offered`. Local diagnostics flag writes also trigger the immediate
hint lookup described above. Personal erasure still checks and removes the
owner's `client_error_reports`, even while the rollout is off; existing reports
and counters continue to TTL-expire.

The proving tests are:

- `crash_default_off_footprint_has_no_export_or_network_commands`
- `crash_worker_without_destinations_returns_without_database_commands`
- `consent_get_warm_default_off_adds_zero_commands_and_enabled_rollout_is_bounded`
- `flag-off telemetry-enabled first-time and opted-in users match main across remounts without preference GETs`
- `public crash hint absent prevents notice preference reads`

## Closed data contract and boundaries

`shared/crash-report.ts` and the backend `CrashReportV1` define schema 1. Each
client builds a new record from finite categories and bounded locations. The
server denies unknown fields at every nested level, validates patterns and time,
and derives severity, fingerprint and environment. No raw error object or stack
is forwarded. Allowed fields are:

- Random event UUID, capture time, platform, kind and finite exception/signal class.
- App version/build, artifact UUID, coarse OS version and browser/Hermes runtime.
- Up to 64 JS frames: same-origin `assets/*.js` or fixed bundle/placeholder names
  and bounded line/column. Function names, origin, query and fragment are omitted.
- For iOS native crashes only: up to 16 image UUID/basename/size entries and 64
  crashed-thread image-index/relative-offset frames. Absolute addresses are omitted.

Messages, NSException reasons, raw stacks, causes, function names, URLs, absolute
paths, registers, custom data, headers, tokens, email, device names, IP and user
agent are never report fields. The authenticated ingestion wrapper has account
and revision authority; these are kept only in the local owner-linked row.
The vendor projection has no account/user/IP/UA/cookie/request identifiers,
breadcrumbs, message value, server name or arbitrary extra fields. Debug and
application tracing expose only fixed outcomes/platform/kind metadata.

## Native lifecycle, buffer and purge proofs

All enabling, arming, disarming and report conversion starts in module
functions called from JavaScript. Module `OnCreate` additionally enforces raw-file
retention before JS. It obtains the pending-file location from PLCrashReporter's
public `crashReportPath()` API; no PLCrashReporter path is reconstructed by NyxID.
Missing binding or raw authority, or an authority/epoch mismatch, deletes the
file unread. A known mtime older than 24 hours also deletes it unread. Otherwise,
the minimal in-memory parse checks capture-time customData `e`/`j`; a missing or
mismatched epoch or a JS fatal duplicate is discarded without retaining parser
data. When mtime is unavailable, the module emits the
closed `age_unknown` metadata notice (`nyx-crash: raw report age unknown
bytes=<n|unknown>`) and parses an authorized report; it never silently discards
it. The parsed report timestamp is then checked: a missing timestamp continues
safely to the sanitizer. `captured_at` uses the parsed timestamp when present,
otherwise the pending file mtime, otherwise collection time. A timestamp or
fallback older than 24 hours or more than five minutes in the future is discarded
as `expired`. The same decision order and 24-hour age bound apply in `collect`.
`initialRawReportDecision`
and `parsedCrashTimestampDecision` have portable tests for every branch and their
boundaries. There are no AppDelegate or SceneDelegate hooks, so app-delegate and
UIScene lifecycles use the same path.

After auth restoration and device-choice hydration, a fresh permission read
selects a binding `{accountId, revision, epochId}`. A matching stored binding keeps
its epoch and re-enables the reporter in the new process. Other bindings are
purged before a fresh epoch is armed. A transport failure during permission
refresh pauses capture without deleting the binding or pending records. Pause
clears native capture authority and custom data even in an already-enabled
process. On iOS it also converts any matching capture-time raw report locally
and purges the raw file through `collect`; only bounded sanitized records survive
an offline launch. A later successful refresh can re-arm the same revision and drain
retained records within the existing bounds. Explicit denial still purges. Logout, account switch, withdrawal, remote
denial or kill switch disarms and purges; transient unknown availability pauses.
A session-restore timeout or failure with no authenticated account also pauses,
retaining the binding and records until the same account can be restored.
Only explicit sign-out or a definitive different authenticated account authorizes
an account-transition purge. A→B→A and
withdrawal/regrant cannot adopt a predecessor's records.

The iOS directory is Application Support/nyx-crash, excluded from backup with
complete-until-first-user-authentication file protection. Android uses its
no-backup directory. Only sanitized fatal/native records enter the bounded
buffer: at most three records, 256 KiB total, 32 KiB per record, 24 hours old.
iOS trims by each record's `captured_at`, allowing at most five minutes in the
future. Missing or unparseable capture times drop only that record; copying an
old report into the buffer does not extend its lifetime. JS revalidation also
rejects records individually and keeps the binding and other valid records.
Writes are synchronous, atomic and fsynced before native return. JS revalidates
both the binding and the closed record before upload. Successful sends remove
the record. Only report-specific refusals with a NyxID JSON error envelope and
numeric code drop that record: 400, 404 (except 13400), 409, 410, 413, 415 and 422.
Code 13400 disarms/purges; 13401 pauses and keeps the record. Authentication and
transport refusals (401, other 403s, 407 and 408), 429, and envelope-less
proxy/captive-portal responses pause and retain it. Diagnostics requests retain
the normal single 401 token-refresh retry, but never invalidate the session on
authentication failure. An expired token therefore removes a buffered record
only after a successful retry; a failed refresh keeps it for later. Other failed
sends may survive only within that consent-bound buffer and its limits.
Interrupted atomic-write staging files are deleted on the next buffer trim or
disarm, including iOS temporary files and Android AtomicFile sidecars. They are
never adopted as records. The Swift filesystem proof checks this cleanup while
preserving committed records and the raw-report directory for its explicit purge.

PLCrashReporter uses a BSD signal handler, no symbolication and
`shouldRegisterUncaughtExceptionHandler = false`, with a 4 MiB raw report cap
shared by capture and parsing (`crashRawReportMaxBytes`). Real Release simulator
reports measured about 375 KB; the raw cap provides headroom and is separate
from the sanitized buffer's 32 KiB per record / three records / 256 KiB total /
24-hour limits. `rawCrashReportDecision` permits parsing only nonempty data up
to the raw cap. Every discard emits only
`nyx-crash: raw report discarded reason=<oversize|unparseable|unauthorized|epoch_mismatch|expired|js_duplicate|reporter_unavailable> bytes=<n|unknown>`.
The separate closed `crashRawNoticeLogLine` emits the `age_unknown` notice while
processing continues. `crashDiscardLogLine` accepts only the closed discard-reason
enum and an optional integer size; the module logs its result with
`NSLog("%@", line)`.
The parsed timestamp is a nil-safe cross-check: `captured_at` uses it when present,
then the pending file's modification time, then collection time. The same
24-hour/five-minute window applies to whichever timestamp is selected.
Report data, paths, identifiers, registers and exception text cannot enter this
formatter.
When the reporter could not be created, the fallback purge logs only if it
removed a raw file; an empty or absent directory produces no discard log.
The raw file can contain crashed-thread register values inside the protected app
sandbox until the next launch. Registers never enter the sanitized buffer,
NyxID or Sentry. The PLCrashReporter signal handler cannot be unregistered within a process.
A crash after pause/disarm still writes a raw file. From that crash until the
next launch it sits in the protected, backup-excluded app sandbox. Pause removes
the persisted raw-capture authority marker while preserving the binding and
sanitized records; disarm removes both. At the next launch, module initialization
deletes such an unauthorized raw file unread. If the app is never launched again,
the file remains on the device. Registers never leave the device.

`pendingCrashDecision` is a pure function: only an equal nonempty epoch and no
`j` marker permits native conversion. The pure `consumePendingRaw` function is
shared by module `OnCreate` and the JS-driven `collect` path. It converts an
authorized report and persists the sanitized record before its single explicit
purge. Unauthorized, epoch-mismatched or expired reports are purged without
loading bytes; oversized, unparseable, timestamp-expired and JS-duplicate
reports are parsed only as needed for their closed decision and are then
purged. `collect` reads the already sanitized records after this consume step.
No binding calls `disarm`, which purges the raw file unread. `disarm` also
purges binding and all sanitized records. These are the container-inspection
purge points for simulator/device review.
Native sanitization selects the thread marked `crashed`. If an externally
delivered signal leaves no crash marker, it selects the thread whose frames
contain the signal's faulting address, then falls back to the first thread.
The fault address is used only for local selection and never enters the record.

React Native's previous ErrorUtils fatal handler runs after synchronous
sanitization and `recordFatalJs`. That write sets custom data `{e, j: eventId}`.
RN's subsequent RCTFatal SIGABRT therefore carries `j` and the pure decision
function drops its native report, preventing double counting. Nonfatal errors
and development RN/Hermes unhandled-rejection callbacks chain the existing handlers.
Release Hermes and fallback rejection trackers use capture-only callbacks; they
do not invoke RN's development rejection logging/native-exception path.

Automated proofs run in `pnpm test`: `native-crash.test.js` compiles/runs the
production Swift decision and sanitizer functions with sentinel-laden inputs on
macOS and Linux (`swift:6.0`), plus the complete mobile CI command sequence on
Ubuntu 24.04 with Swift 6.4.2, Node 22 and pnpm 10.17.1. `ParsedNativeCrash` contains only the values needed
for mapping. `sanitizedNativeCrash` performs thread selection, image/offset
mapping, frame/image limits, basename/UUID/signal normalization, version/OS and
capture-time validation, then the closed `sanitizedNativeRecord` boundary.
The proof generates 40 unmarked threads with 64 frames each and 600 images with
long simulator/app paths, plus raw register metadata that has no field in the
parsed value type. It measures the serialized fixture (at least 375,000 bytes),
passes that size through the production raw decision and maps the parsed values
through the production sanitizer. It asserts the exact output allowlist, image
and frame bounds, relative offsets, lowercase UUIDs and installed version/build,
and checks serialized output for sentinel paths, container IDs, absolute
addresses and register values. Cap-minus-one/cap/cap-plus-one/zero decisions and
the exact metadata-only log format are also CI-proven. Existing proofs cover
marked/fault-address/first/empty thread selection, epoch/JS-duplicate decisions,
JS message/reason/register/customData/absolute-path rejection and the closed
native output boundary. `crashReporting.test.ts` tests native-record JS
revalidation, Hermes/JSC parsing, request bodies, fatal-handler ordering,
relaunch rearming and lifecycle purges.

CI does not exercise the Apple-only PLCrashReport parser or the PLCrashReport
adapter itself. The adapter copies parser fields into `ParsedNativeCrash`; the
portable proofs exercise the decision, mapping and log format after that boundary.
Adapter behavior with an actual crash and raw-file deletion are device/simulator
proofs. The existing explicit `collect` purge and `defer` cover over-cap and
unparseable data as well as the other branches listed above; Linux cannot run
those module purge calls. The PM's app-container inspection verifies deletion.
A Release simulator compile proves integration, not crash delivery or deletion.

## Storage, admission and operations

`POST /api/v1/users/me/crash-reports` accepts uncompressed JSON within 64 KiB,
under device/account consent headers. It returns 202 accepted/duplicate with a
local report ID and no-store. Cluster limits apply before parsing: 5/minute and
50/day/account, 30/minute/trusted client IP and 600/minute/global. Account windows
are checked first, then IP, then global, so a locally exhausted client cannot
consume the shared global budget. IP is only a
rate-window key, never a report field. Unavailable capture returns 13400/404;
missing or stale diagnostics consent returns 13401/403 only after a successful
linearizable read confirms denial. Consent reads distinguish permitted, denied
and unknown: timeout or database failure is unknown, including in the pre-send
export check. Unknown readiness or consent lookup failures return the existing
transient 503 `security_readiness_unavailable`. A policy-2 acknowledgement while
the offer is unknown returns that same 503; definitively not offered returns
validation error 400. The exporter retains an unknown-consent row as pending
with a ten-second backoff (`consent_unknown`), without sending. Mobile
pauses on 13401/503 and disarms only on definitive 13400. The web session stops
only for 13400/13401; a later available permission refresh clears that stop.
Unknown/5xx permits a later refresh. Capture timestamps are capped at forty bytes
before parsing and stored/exported as normalized UTC RFC3339 values.

`client_error_reports` has UUID IDs, a unique owner/event index, owner revision,
closed report, derived fields, receive/expiry timestamps and fenced export state.
Retention is 1–30 days (default 30); every read filters live expiry as well as the
Mongo TTL. Personal erasure deletes these owner-linked rows through the existing
erasure inventory and admission barrier. The self-serve account export includes
only report metadata: received_at, expires_at, environment, severity,
report.platform, report.kind, report.exception_class, report.native_signal,
report.app_version, report.app_build, report.os_version and report.runtime;
counters and release-registry rows are excluded. `client_error_report_counters` retains
only anonymous hourly platform/outcome counts, with a 30-day TTL. After rate
limiting, insert failures count as `consent_denied` for confirmed consent denial,
`invalid` for validation errors, and `unavailable` for unknown consent or other
database/internal failures. Pre-admission eligibility refusals remain uncounted.

NyxID keeps its fingerprint for local grouping only; Sentry receives no custom
fingerprint, preserving symbolicated grouping. Web uses the browser context;
iOS/Android include OS names. Mobile omits an unavailable Hermes version.

The exporter claims batches of at most sixteen using database-time leases and
random fences. It rechecks readiness, flag, revision and the 24-hour export
deadline, then persists dispatching before network I/O. 2xx settles exported;
429/503 and proven connect failures retry with bounded backoff up to one hour
within the deadline. Timeout or ambiguous delivery becomes unknown and is never
resent. Expired dispatch leases become unknown. Other provider refusals become
failed. A replica with no matching destination releases the lease as pending with
a short backoff, allowing a configured replica to export it before the deadline.
An export interval of zero disables all export work, including existing pending
rows (which TTL-expire), and stores new rows as disabled even with a DSN.
With no export destinations configured, the worker exits without database
commands; rows already pending after the last DSN is removed wait for TTL expiry.
The admin health explicitly shows Export disabled. Stale workers cannot settle successor claims. Missing platform DSN stores
reports with export disabled, with no backlog awaiting later configuration.

The exporter runs under the process shutdown runtime. Each admitted pass holds
one background-work guard from the interval tick through the stale-lease sweep,
claim, send and fenced settlement. A draining replica checks before every
further claim in the sixteen-row batch and finishes its current send and
settlement before releasing that guard. A send completed during the drain
settles normally, including `exported` on success. For the configured
`SHUTDOWN_DRAIN_TIMEOUT_SECS` budget `t`, the drain reserves `min(t / 10, 5 s)`
for cleanup, and the work deadline is `t` minus that reserve.
`SHUTDOWN_DRAIN_TIMEOUT_SECS` overrides the default. With today's default of
`t = 25 s`, the cleanup reserve is 2.5 seconds and the work deadline is 22.5
seconds, which contains the ten-second exporter HTTP timeout. If a shorter
configured budget or second signal aborts an in-flight send, its persisted
`dispatching` intent and lease remain. A later expired-lease sweep marks it
`unknown` and never resends it. An expired or superseded lease cannot settle a
report.

## Sentry configuration and erasure acceptance

Before configuring either server DSN, the owner must accept this erasure
approach. Configure **separate web and mobile projects**, project data retention
**30 days or less**, **Prevent Storing of IP Addresses** enabled, and
**server-side data scrubbing** enabled. Verify these settings in the deployed
projects; source code cannot attest vendor administration settings.

Vendor events contain no account/user/IP/UA/cookie identifiers. NyxID local rows
are the only event-to-person mapping; personal erasure deletes them. There is no
supported per-event Sentry deletion guarantee on which this implementation
relies. Vendor events expire under the configured project retention. Do not
enable the exporter without owner acceptance of this unlinking/retention model.
DSNs require HTTPS and a single numeric project path; invalid configuration
fails startup without printing secrets. HTTP redirects and proxy inheritance
are disabled. Export requests have a ten-second timeout and only fixed Sentry
protocol headers.

## Private build artifacts

Web Vite builds use a provided `CRASH_ARTIFACT_ID` UUID, package version and
build ID, and hidden source maps. An explicitly malformed UUID fails the build;
local builds without an ID generate a random UUID. The publish workflow derives
one deterministic UUIDv5 from repository, commit SHA and workflow run ID in a
single prerequisite job and passes it to both architecture builds. Only the
amd64 frontend build receives the optional upload secret, so symbols upload once.
Cross-architecture reproducibility was verified at `8cf56b4aa`: with identical
build arguments, the `linux/amd64` and `linux/arm64` frontend builder stages
produced byte-identical JS bundles, all 456 source maps and the collected
crash-artifacts tree; with a pinned `SOURCE_DATE_EPOCH`, all 1,494 output files
were identical. Without a pinned epoch, as in CI, only the upstream
release-integrity manifest's `generated_at` timestamp differed, which no crash
artifact references, so arm64 frames symbolicate against the maps uploaded once
from amd64.
The build collector moves all maps out of dist into
`crash-artifacts/web/<artifact>/`, copies matching assets, writes a manifest and
fails if a map or sourceMappingURL remains public. nginx also rejects `.map`.
The runtime image copies only dist. The builder installs the artifact uploader
only when the optional secret file is present and nonempty. Optional Docker secret
`crash_artifacts_sentry` is a JSON object of the three web build-only variables
listed in ENV.md; single-line and multi-line JSON are supported. The publish
workflow passes it through `secret-envs` only for frontend/amd64 when nonempty;
the secret input is empty on every other leg and when unset. A configured upload
failure fails that image build. No token uses a VITE prefix or enters a runtime layer.

Mobile scripts generate `NYXID_CRASH_ARTIFACT_ID` before prebuild. iOS archive
sets SOURCEMAP_FILE for the Hermes composed map and copies main.jsbundle, map,
dSYMs and dwarfdump UUIDs immediately after archiving. Android copies its release
bundle and composed map; this provides JS symbolication only. Each manifest is
under `mobile/release/crash-artifacts/<platform>/<version>-<build>-<artifact>/`.
These directories are gitignored and must be retained privately by release ops.
Reports use installed native version/build, not package metadata or an OTA label.
Without symbol-upload secrets, missing bundles, composed maps, dSYMs or UUIDs
produce a warning and the store build continues, including iOS IPA export.
Once upload is configured, missing artifacts and upload failures fail the build;
partial secret configuration also fails.

`tools/crash-artifacts` pins its own CLI and lockfile. Upload does nothing unless
the auth token, organization and matching project are configured together; a
partial configuration fails the build. It uploads release/dist source maps and
iOS debug files without echoing credentials. Sentry releases are
`nyxid-web@<version>+<artifact>` and `nyxid-ios@<version>+<build>` or `nyxid-android@<version>+<build>`. Keep the tools
package installed before a mobile build (`npm ci --prefix tools/crash-artifacts`).

## Rollout and rollback

Deploy all backend readers/workers, then web and the new mobile binary. Verify
`diagnostics:crash-reports` serving readiness, enable a staff cohort, verify
counts/details and health, and widen. Policy 2 is advertised only when readiness
serves, the acting person's flag is enabled and the global kill switch is inactive.
Provisioning Sentry, owner acceptance, vendor settings, App Store crash
data disclosure and a TestFlight device proof are release-operator prerequisites.

An explicit global flag override off is the kill switch, including narrower
user/org enables. Ingestion refuses, clients purge when they observe denial,
and export discards pending records. Disable first for binary rollback; older
binaries ignore additive fields and new collections. Diagnostic reports and
counters TTL-expire; the release registry persists with shared attribution. No migration
of existing analytics consent is performed.
After binary rollback, an old replica's personal-erasure inventory does not
include `client_error_reports`: a person erased by that replica retains those
rows until TTL expiry (at most 30 days). Operators needing immediate removal
must erase again after upgrading, or delete those rows by owner.

## Third-party components

- **PLCrashReporter 1.12.0**, Microsoft, **MIT** license.
  Upstream: <https://github.com/microsoft/plcrashreporter>. Pinned by the local
  Expo module podspec and Podfile.lock. No vendor SDK or direct device egress.
- **@sentry/cli 3.8.2**, Sentry, package.json SPDX **FSL-1.1-MIT**.
  Upstream: <https://github.com/getsentry/sentry-cli>. Build-tool dependency only;
  the package license is FSL with its stated future MIT conversion, not BSD.

## Linux portability proof

The same production `CrashDecision.swift` and test executable pass with
swift-corelibs-foundation in the official `swift:6.0` Linux container:

```sh
docker run --rm -v "$PWD/mobile:/w:ro" -w /w swift:6.0 bash -lc \
  'swiftc modules/nyx-crash-reporter/ios/CrashDecision.swift modules/nyx-crash-reporter/tests/main.swift -o /tmp/nyx-crash-proof && /tmp/nyx-crash-proof'
```

`pnpm test` always runs the Swift proof; it never skips for a missing compiler.
The GitHub-hosted image inventory lists Swift 6.4.2 for both
[Ubuntu 24.04](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2404-Readme.md#language-and-runtime)
and [Ubuntu 26.04](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2604-Readme.md#language-and-runtime).
The [runner image label table](https://github.com/actions/runner-images#available-images)
maps `ubuntu-latest` to the hosted Ubuntu image; both sides of the November 2026
label transition include the Swift compiler.

## Release identity registry

`client_release_registry` stores operational release metadata with shared
attribution and is never exported. Its unique key is platform/version/build/artifact; UUID IDs and
BSON registration timestamps follow the model conventions. Optional
`registered_by` is shared administrative attribution, not report ownership:
only the opaque admin ID is retained, never names or emails. Personal erasure
does not delete operational release identities; this retained-with-reason
disposition is recorded in the personal-erasure inventory (the existing
`client_error_reports` is owner-linked personal diagnostic data;
`client_error_report_counters` is anonymous aggregate operational data).

At backend startup and every fifteen minutes, a tracked shutdown worker checks
the diagnostics rollout. Only global or cohort rollout permits it to fetch the
live manifests from configured frontend origins and register valid identities
with source `frontend_manifest`, even before the first crash arrives. Off,
killed, or unavailable rollout checks cause no outbound requests or registry
writes. Each valid identity is looked up before registration, so a known release
causes no steady-state write.
There is no new environment variable. Each pass holds a `tick` work guard through
fetch and registration; drain stops further passes and checks before the next
origin. It reuses the hardened fetch, sixty-second positive/negative origin cache,
single-flight, and closed failure logging described below. Invalid or unreachable
manifests register nothing.

After a consent-verified report is successfully inserted, a background task
checks `crash-build.json` for unknown web identities at
`FRONTEND_URL` and each `FRONTEND_URL_ALIASES` origin only. Report fields never
form URLs. Production requires HTTPS; configured development HTTP is allowed.
Requests disable redirects and carry no account data, authorization or cookies,
with a three-second timeout, 4 KiB response cap and closed JSON schema. A
matching version/build/artifact is registered from `frontend_manifest`. Positive
and negative fetch results are cached per configured origin for sixty seconds
with single-flight: distinct untrusted artifact IDs cannot increase the fetch
rate. Ingestion never waits for these requests. Each failed fetch emits one
metadata-only warning with a closed reason code (`transport`, `status`,
`oversize`, `invalid_json` or `mismatch`) and the origin host only; report fields,
artifact IDs, response bodies and URL paths/queries never enter that warning.

The request registers this background task as detached shutdown work before it
starts. Ingestion still returns without waiting for manifest I/O, while a
process drain waits for an in-flight registration within its work deadline.

An administrator registers each TestFlight/Play build using the Unverified
releases action in `/admin/crash-reports`, or admin-only first-party-human
`POST /api/v1/admin/crash-reports/releases` with
`{platform, app_version, app_build, artifact_id?}`. Operators can inspect unverified
counts but cannot register. Registration first upserts the registry identity,
then writes a metadata-only audit event with the actual outcome and returns
`{status: "registered" | "already_registered"}`. Repeats preserve the existing
row and attribution, including concurrent identical upserts that encounter the
unique identity index. A failed registry write never audits success. Mobile cannot
register automatically because store binaries have no server-reachable manifest.
Verification uses a query-time registry lookup: version totals contain only
verified records, and Unverified releases shows the others. Later registration
needs no report backfill. Mobile verification binds platform/version/build;
web additionally binds the artifact. Individual reports remain inspectable.

Every Sentry event explicitly carries
`sdk.settings.infer_ip = "never"`, with SDK name `nyxid.crash-export` and the
backend package version, and has no `user` object. This avoids JavaScript/Cocoa
legacy inference; see [Sentry SDK event payload settings](https://develop.sentry.dev/sdk/data-model/event-payloads/sdk/).

## Intentional behaviour

- GPC also disables client analytics, matching the server and narrowing collection.
- NSException crashes carry only the abort stack. The exception handler is disabled for privacy.
- Capture is unarmed during the permission round trip and after a failed fetch.

## Hermes artifact validation proof

The pinned `sentry-cli 3.8.2` accepts the real Release Hermes bytecode bundle
(magic `c61fbc03`) and its composed source map with the existing upload flags.
A local metadata-only Sentry API stub was used with the Release simulator files
produced with `SOURCEMAP_FILE` set. The command was
`tools/crash-artifacts/node_modules/.bin/sentry-cli sourcemaps upload /private/tmp/ws4-r5-evidence/hermes --release nyxid-ios@1.0.4+45 --dist 45 --url-prefix app:/// --validate --strict`.
It exited 0 and reported “Found 2 files”, “Analyzing 2 sources”, “Validating
sources”, “Bundled 2 files for upload” and a script/map pair at
`app:///main.jsbundle` / `app:///main.jsbundle.map`. Validation executes on the
bytecode and composed map before the stub's assembly response; it is not a
mock of the CLI. No vendor or production project was contacted. The upload
flags remain unchanged. See the [Sentry CLI source-map upload documentation](https://docs.sentry.io/platforms/javascript/sourcemaps/uploading/cli/)
and [React Native Hermes source maps](https://docs.sentry.io/platforms/react-native/sourcemaps/uploading/hermes/).

# Crash and error reporting

This is the normative contract for issue #2281. The `diagnostics:crash-reports`
feature flag and matching fleet readiness capability default off. NyxID stores
sanitized reports itself and provides `/admin/crash-reports` to first-party
human administrators and operators. Sentry export is optional and server-only.
Counts are **reported events**, not a crash-free percentage, unique people or
complete incident coverage.

## Coverage and consent

Web automatically observes window errors, resource failures, unhandled promise
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

Policy-1 users keep categorical analytics. Legacy stored device choices mean
policy 1; a PUT without `policy_version` still records `1`. New clients negotiate
`current_policy_version` before sending `2`. Web offers the updated consent
banner and a Privacy review dialog. Mobile offers an account-bound one-time
Review / Not now alert and a Privacy review row. Not now retains the old choice.
Local withdrawal remains usable during network failures.

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

All enabling, arming, disarming and pending-report processing starts in module
functions called from JavaScript. There are no AppDelegate or SceneDelegate
hooks, so app-delegate and UIScene lifecycles use the same path.

After auth restoration and device-choice hydration, a fresh permission read
selects a binding `{accountId, revision, epochId}`. A matching stored binding keeps
its epoch and re-enables the reporter in the new process. Other bindings are
purged before a fresh epoch is armed. Logout, account switch, withdrawal, remote
denial, kill switch or unavailable diagnostics disarms and purges. A→B→A and
withdrawal/regrant cannot adopt a predecessor's records.

The iOS directory is Application Support/nyx-crash, excluded from backup with
complete-until-first-user-authentication file protection. Android uses its
no-backup directory. Only sanitized fatal/native records enter the bounded
buffer: at most three records, 256 KiB total, 32 KiB per record, 24 hours old.
Writes are synchronous, atomic and fsynced before native return. JS revalidates
both the binding and the closed record before upload. Successful sends remove
the record; denied authority purges it. Other failed sends may survive only
within that consent-bound buffer and its limits.

PLCrashReporter uses a BSD signal handler, no symbolication and
`shouldRegisterUncaughtExceptionHandler = false`, with a 256 KiB raw report cap.
The raw file can contain crashed-thread register values inside the protected app
sandbox until the next launch. Registers never enter the sanitized buffer,
NyxID or Sentry. The process signal handler is enabled once; disarming clears
custom data and binding so any later raw report is ineligible.

`pendingCrashDecision` is a pure function: only an equal nonempty epoch and no
`j` marker permits native conversion. `NyxCrashReporterModule.collect` always
purges raw data in its `defer`, and explicitly purges **before returning** any
sanitized records. This covers successful parsing, epoch mismatch, corrupt or
oversized data and a JS fatal duplicate. No binding calls `disarm`, which purges
the raw file unread. `disarm` also purges binding and all sanitized records.
These are the container-inspection purge points for simulator/device review.

React Native's previous ErrorUtils fatal handler runs after synchronous
sanitization and `recordFatalJs`. That write sets custom data `{e, j: eventId}`.
RN's subsequent RCTFatal SIGABRT therefore carries `j` and the pure decision
function drops its native report, preventing double counting. Nonfatal errors
and RN/Hermes unhandled-rejection callbacks chain the existing handlers.

Automated proofs run in `pnpm test`: `native-crash.test.js` compiles/runs the
production Swift decision and sanitizer functions with sentinel-laden inputs.
It tests match/mismatch/no-binding/JS-duplicate decisions, rejects JS message,
reason, registers, customData, absolute paths and extra keys, and proves native
output is a closed allowlist. `crashReporting.test.ts` tests native-record JS
revalidation, Hermes/JSC parsing, request bodies, fatal-handler ordering,
relaunch rearming and lifecycle purges. The PM's simulator container inspection
provides runtime raw-file deletion evidence; a Release simulator compile alone
does not establish device signal-delivery behavior.

## Storage, admission and operations

`POST /api/v1/users/me/crash-reports` accepts uncompressed JSON within 64 KiB,
under device/account consent headers. It returns 202 accepted/duplicate with a
local report ID and no-store. Cluster limits apply before parsing: 5/minute and
50/day/account, 30/minute/trusted client IP and 600/minute/global. IP is only a
rate-window key, never a report field. Unavailable capture returns 13400/404;
missing or stale diagnostics consent returns 13401/403. Clients purge on either.

`client_error_reports` has UUID IDs, a unique owner/event index, owner revision,
closed report, derived fields, receive/expiry timestamps and fenced export state.
Retention is 1–30 days (default 30); every read filters live expiry as well as the
Mongo TTL. Personal erasure deletes these owner-linked rows through the existing
erasure inventory and admission barrier. `client_error_report_counters` retains
only anonymous hourly platform/outcome counts, with a 30-day TTL.

The exporter claims batches of at most sixteen using database-time leases and
random fences. It rechecks readiness, flag, revision and the 24-hour export
deadline, then persists dispatching before network I/O. 2xx settles exported;
429/503 and proven connect failures retry with bounded backoff up to one hour
within the deadline. Timeout or ambiguous delivery becomes unknown and is never
resent. Expired dispatch leases become unknown. Other provider refusals become
failed. Stale workers cannot settle successor claims. Missing platform DSN stores
reports with export disabled, with no backlog awaiting later configuration.

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

Web Vite builds generate a random artifact UUID, package version and build ID,
and hidden source maps. The build collector moves all maps out of dist into
`crash-artifacts/web/<artifact>/`, copies matching assets, writes a manifest and
fails if a map or sourceMappingURL remains public. nginx also rejects `.map`.
The runtime image copies only dist. Optional Docker secret
`crash_artifacts_sentry` is a JSON object of the three web build-only variables
listed in ENV.md. No token uses a VITE prefix or enters a runtime layer.

Mobile scripts generate `NYXID_CRASH_ARTIFACT_ID` before prebuild. iOS archive
sets SOURCEMAP_FILE for the Hermes composed map and copies main.jsbundle, map,
dSYMs and dwarfdump UUIDs immediately after archiving. Android copies its release
bundle and composed map; this provides JS symbolication only. Each manifest is
under `mobile/release/crash-artifacts/<platform>/<version>-<build>-<artifact>/`.
These directories are gitignored and must be retained privately by release ops.
Reports use installed native version/build, not package metadata or an OTA label.

`tools/crash-artifacts` pins its own CLI and lockfile. Upload does nothing unless
the auth token, organization and matching project are configured together; a
partial configuration fails the build. It uploads release/dist source maps and
iOS debug files without echoing credentials. Sentry releases are
`nyxid-web@<version>+<artifact>` and `nyxid-mobile@<version>+<build>`. Keep the tools
package installed before a mobile build (`npm ci --prefix tools/crash-artifacts`).

## Rollout and rollback

Deploy all backend readers/workers, then web and the new mobile binary. Verify
`diagnostics:crash-reports` serving readiness, enable a staff cohort, verify
counts/details and health, and widen. Policy 2 is advertised only when readiness
serves. Provisioning Sentry, owner acceptance, vendor settings, App Store crash
data disclosure and a TestFlight device proof are release-operator prerequisites.

An explicit global flag override off is the kill switch, including narrower
user/org enables. Ingestion refuses, clients purge when they observe denial,
and export discards pending records. Disable first for binary rollback; older
binaries ignore additive fields and new collections, which TTL-expire. No
migration of existing analytics consent is performed.

## Third-party components

- **PLCrashReporter 1.12.0**, Microsoft, **MIT** license.
  Upstream: <https://github.com/microsoft/plcrashreporter>. Pinned by the local
  Expo module podspec and Podfile.lock. No vendor SDK or direct device egress.
- **@sentry/cli 3.8.2**, Sentry, package.json SPDX **FSL-1.1-MIT**.
  Upstream: <https://github.com/getsentry/sentry-cli>. Build-tool dependency only;
  the package license is FSL with its stated future MIT conversion, not BSD.

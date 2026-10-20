# Build input and shipped artifact security

`security/build-input-pins.json` records the authenticated GitHub commit API
resolution of each third-party Action and the HTTPS Registry v2 resolution of
each Docker base index. The image receipt records its supported platforms; the
SHA-256 of the raw index bytes matched its registry digest. These receipts do
not claim upstream image signatures. Dockerfiles retain readable version tags
alongside the immutable index digests. Dependabot proposes updates; changed
pins also require refreshed resolution receipts and validation.

The repository toolchain is Rust 1.98.1. dtolnay's Action uses the reviewed
version-specific commit and supported component inputs; its pinned implementation
selects 1.98.1. Rust builds enforce Cargo.lock, including dependency-cache builds.
The separately locked machine context spike retains its own lockfile.

`scripts/bootstrap-build-tool.py` installs cargo-dist 0.30.0 from a committed
archive digest, without executing its downloaded installer shell. The focal
arm64 release container uses the committed rustup 1.28.2 executable digest to
install exact Rust 1.98.1; rustup verifies official toolchain component hashes.
Host runners use the pinned Rust Action. npm 11.21.1's archive is pinned by its
registry SHA-512 integrity and checked before extraction or execution. The
bootstrap script bounds downloads and rejects unsafe archive members.

The release and cache workflows treat generated matrix arguments as data and
never execute generated installer/package shell commands. The reviewed release
dependency script owns Linux package installation. `check-build-inputs.py`
rejects mutable Actions/bases, missing receipts, unlocked Rust builds and
regenerated cargo-dist workflows that restore unchecked installers. Run it
after `dist init`; `allow-dirty=["ci"]` permits the reviewed workflow patch,
and does not exempt that policy check.

The Glama reference image builds the supplied reviewed source context with a
pinned Rust image and Cargo.lock. Operators copying this definition into its
external UI must select the same immutable repository revision. It no longer
clones a moving default branch or executes an unverified Rust installer.
Use `scripts/build-local-image.py --file Dockerfile.glama --tag <local-tag>`
from a clean checkout to export the exact committed source and supply both the
full commit and commit epoch. Direct external builders must supply the same
`NYXID_GIT_HASH` and `SOURCE_DATE_EPOCH` arguments from that committed export.
An empty, abbreviated or conflicting identity fails the build.

Application images retain `org.opencontainers.image.revision` and
`io.nyxid.source-date-epoch` labels. These labels are checked against an
independent source export and the actual final executable or assets. Backend,
CLI, updater and mcp-demo executables retain the shared versioned identity
section after stripping. The verifier requires the exact package/version,
architecture/ABI, full commit, epoch and `exported` source state; local
`git-unverified`, dirty and unknown identities cannot pass a publication check.
The scratch updater additionally requires a static ELF without an interpreter.

The frontend emits `build-identity.json` from the same commit/epoch used for
its credential manifest. Final-image checks require matching update metadata,
existing initial assets, the complete final composition file set and hashes,
and the committed frontend lock hash. Developer builds can remain explicitly
unverified. Identity metadata identifies a build; authenticated signatures and
actual-artifact SBOMs remain separate required release controls.

`Artifact Packaging` is required by PR `CI Pipeline` and runs weekly with
read-only permissions and no private context or secrets. It extracts the actual
backend/frontend runtime images on both native Linux architectures, checks
credential-page parity and verifies each final image identity. `Shipped Image
Identity` covers node, machine, updater production, machine-node-only,
machine-hosted's public desktop stage and mcp-demo on both architectures.
`CLI Archive Identity` builds the real stripped release archives on all four
planned Darwin/GNU targets and inspects their packaged executable bytes.
Changes to these verifiers and public docs build inputs select the PR jobs too.
The public image publisher repeats identity inspection on each exact built
digest before attestation/tag promotion; the CLI release producer checks its
stripped archive before uploading it.

PR evidence for `cli/Dockerfile.machine-hosted` ends at the real public
`desktop` stage. A publisher with private CMA context must invoke
`scripts/check-image-identity.py --component machine-hosted` on the final
`hosted` image after all private layers and before publication, supplying the
same exported source root, commit, epoch, native architecture and locked
`nyxid-artifact-info` inspector. Hosted deployment uses the operator path in
`docs/MACHINE_HECA_CMA_IMAGE.md`, not a CI publisher. `build-local-image.py`
now requires this check for every reviewed application target, builds the
inspector from the exact source export, and applies the requested tag only
after verification. Its existing execution receipt records the image ID,
component/platform, check result and tagging result. Missing or failed checks
return non-zero without applying a tag; no skip/alternate-checker option exists.
Unknown image/stage combinations reject; the updater's explicit `e2e` target
retains a distinct test-only receipt. PR CI exercises the operator sequence
against the real public hosted desktop on both native architectures. Private
patch bytes remain external, and final private-image acceptance stays with
the operator's successful receipt and recorded runtime tests.

`Built Image Worklet CSP (amd64/arm64)` runs the existing real microphone,
hashed AudioWorklet, font and WebSocket case plus the enforcing CSP rejection
case against the actual frontend runtime image. The runtime inherits its
packaged default entrypoint and nginx configuration; only writable tmpfs paths
are supplied. Tests read safely extracted image assets, and HTTP byte checks
bind the served HTML/worklet to that same immutable image ID. Browser tools
come from the digest-pinned, native Playwright image in
`frontend/Dockerfile.policy-tests`, with dependencies installed from the
frontend lock without hooks. Its version must match locked Playwright;
Dependabot's weekly frontend Docker/npm updates cover both inputs. Missing,
skipped, retried or failed cases fail the required PR/weekly gate.

`Machine Docker CLI E2E` is required on PRs and weekly, alongside the existing
`Machine Container E2E` sandbox/update coverage. It builds the actual desktop
and node-only images, extracts each real CLI, and drives `node docker start
--machine`, restart and stop/start through the hosted Docker engine. The
node-only image supplies deterministic unsupported capability evidence:
omitted/automatic/require-separated choices must fail with closed stdin before
any fixture request, registration, local config, persistent volume or container.
Both images must register exactly once with explicit `shared-legacy`, advertise
live capabilities/heartbeats, and preserve encrypted custody and their recorded
choice on restart. Only a temporary local WSS fixture and generated CA are used;
its transport comes from the existing updater-test lock. A transparent recorder
executes the real Docker client unchanged and checks that probes receive no
credential environments or identity mounts. Resource inventories reject leftover
probe containers/anonymous volumes; cleanup targets only unique fixture names.
This uses D's existing separation/environment/CA contract without production
image-selection overrides or edits to D's CLI and entrypoint.

The two machine test Dockerfiles deliberately accept `MACHINE_IMAGE` from the
exact same CI build, rather than claiming a preexisting release digest. The
context-spike fixture names an immutable published machine digest. Production
Dockerfiles have no parameterized base-image exception.

`security/test-images.json` pins the historical0.40 machine,0.41 machine/updater
and BusyBox mailbox fixtures. The migration harness pulls its old bytes by
digest before creating the local historical tag expected by the test adapter;
it never fetches that tag. The production harness consumes the pinned fixtures
directly. Registry receipts also cover the already-pinned Mongo8 test image and
context-spike base. `check-build-inputs.py` validates these inputs and Cargo's
parsed dist container metadata, including an actual generated plan supplied by
`--dist-plan`. Both release workflows check the plan before consuming its matrix.
Workflow checks cover both .yml and .yaml. The guard checks the inventoried
image definitions, rather than claiming to statically
resolve arbitrary image construction in application code.

Ubuntu/Debian additions use `scripts/apt-snapshot-install.sh` and the reviewed
`security/apt-snapshots.tsv` catalog. Each root has base, updates and security
suites at one timestamp, with an exact signed InRelease SHA-256 per suite.
The portable selector rejects absent requested roots, duplicate suites/roots,
mixed timestamps, unexpected origins/components and malformed hashes before
APT runs. Every Dockerfile stage that installs APT packages copies the same
installer, selector and catalog. Hosted Chromium selects both noble and
bookworm, retaining its Debian package preference rules.

The installer isolates sources, indexes and archive caches from mutable base
configuration. Distro keyrings verify signatures; pinned metadata authenticates
native-architecture indexes and package archives. Fixed snapshots disable only
Release.Valid-Until freshness, retaining signature and checksum enforcement.
Catalog and production acquisition URLs use HTTPS directly. Every caller now
supplies a pinned transport bundle, avoiding Ubuntu's HTTP-to-HTTPS redirect
for each of the desktop closure's hundreds of small archives. The disposable
tamper-test wrapper alone rewrites its isolated sources to HTTP for byte
injection: HTTP is safe for this integrity test only because the reviewed
SHA-pinned **signed InRelease** authenticates those bytes, followed by its
authenticated native indexes and package checksums. Minimal images need
transport trust before installing CA packages. Every caller carries
`security/apt-transport-ca.pem`, the public
Mozilla bundle extracted by curl as of 2026-09-25, obtained from
`https://curl.se/ca/cacert.pem` and compared with its HTTPS `.sha256` publication.
`security/apt-transport-ca.tsv` records the exact upstream bundle and checksum
URLs, Mozilla source timestamp (`2026-09-25T03:12:01Z` for the initial bundle),
upstream check date, and SHA-256. The portable manifest parser supplies that
reviewed digest to every installer; provenance and bundle travel together.
The installer rejects missing, symlinked or changed bundle bytes before any
APT request. It supplies this file only via `Acquire::https::CaInfo`, requires
peer and hostname verification, and never adds it to the image's system trust
store. Distro `ca-certificates` packages install normally only after the
snapshot metadata, indexes and archives have passed their independent checks.
CI and release-host wrappers use that same checked bundle and installer.

The APT layer job builds only the real Dockerfile instructions through each
stage's last package install. A fresh job-local BuildKit builder reuses identical
desktop/Chromium prefixes in derived test images; no external cache is imported.
Cold package prefixes retain a 2,400-second bound (including archive downloads,
configuration and image loading), with elapsed times and a log tail on timeout.
The timeout has not been raised to accommodate repeated desktop downloads.
Every selected component's
native Packages index must exist even when older APT reports acquisition errors
as warnings. Only amd64/arm64 without foreign architectures are admitted.

Image builds upgrade inherited packages and install explicit additions from
downloaded archives using `--no-download`. Hosted development runners use
`--install-only` for their requested dependencies, with the same authentication
and offline installation; their existing OS is not a shipped image root.
Dockerfile admission requires the image-upgrade invocation. Package records
under `/usr/share/nyxid/build-inputs/apt/` retain snapshot metadata, requested
packages, base/final inventories and downloaded archive hashes. Each record
binds its prior package state and request so inherited image installs preserve
earlier records.

On relevant pull requests, `CI / APT Inputs` calls `apt-inputs.yml`; its result
is required by `CI Pipeline`. The same reusable workflow runs weekly. `APT
Policy` validates catalogs/callers and portable negative tests. Eight `APT
Snapshots (base, arch)` jobs exercise bookworm/trixie/focal/noble on native
amd64/arm64: success, catalog digest mismatch, malformed/bad/absent signatures,
missing/corrupted indexes, corrupted packages and a removed downloaded archive.
Failure cases require unchanged dpkg state; metadata rejection must precede all
package downloads. Success also verifies repeat/install-only calls preserve
earlier package records. Two `APT Dockerfile Layers (arch)` jobs build all twelve
actual package-stage prefixes without cache, plus a direct minimal-Ubuntu
regression with no system CA bundle before or after installation, then inspect
records in containers
with no network. These projections preserve package instructions and source
inputs but stop before application compilation; they are not evidence of final
application image behavior. Logs, exact prefixes and inventories are retained
as seven-day workflow artifacts. These jobs need no repository secrets, OIDC,
registry login or remote publication permissions.

Alpine additions use six reviewed catalogs under `security/apk/`: frontend
runtime, updater builder and updater E2E, each for amd64 and arm64. A catalog
binds the digest-pinned base, native architecture, refresh date, explicit
requested packages, before/after installed inventories, and exact archive
name/version/architecture/SHA-256, a repository-owned durable HTTPS URL and
the original Alpine HTTPS URL as provenance. Both the portable installer parser
and Dockerfile admission reject malformed or duplicate headers/rows, mutable
origins, path traversal, architecture mismatches and absent matrix members.
Signed noarch metadata is retained even when an APK 3 repository install had
recorded a native architecture for the same meta-package.

Product builds never update APK indexes or solve against a mutable repository.
A separate digest-pinned download **stage** checks every catalog archive's
checksum, distro signature and signed `.PKGINFO` identity. Each installing
stage starts from the same pinned base and uses `RUN --network=none` with a
read-only BuildKit bind mount from its download stage. It rechecks the inputs,
replays the complete local transaction into a temporary root containing a
copy of the base APK database and keyring, with scripts/triggers, networking,
repositories and cache disabled. That root's final inventory must match the
catalog before the live image's package database can change; `--simulate`
alone does not prove this. The live install also disables networking,
repository files and cache and independently checks its final inventory.
Inventories and receipts are written
outside that mount. Download layers are neither inherited nor copied into
shipped images; deleting archives in a later layer would still ship their
bytes and is rejected by policy. Unlisted files cannot enter the transaction.
The final installed inventory must match the catalog or the image layer fails.
Successful catalog/base/final records remain under
`/usr/share/nyxid/build-inputs/apk/<catalog-sha256>/`. The updater's production
stage continues to copy only its binary and CA bundle into scratch.

`resolve-apk-candidates.py --arch amd64 --output /tmp/apk-candidates` (and arm64)
is an explicit candidate-refresh tool, never a product build input. It copies
the exact target image's package database/keyring/repositories into an isolated
solver root and runs signature-checked repository resolution with scripts and
APK self-reexecution disabled. It can resolve another architecture without
running foreign binaries. Downloaded archive signatures and identities are
checked before writing candidates; review the resulting `archives.tsv`,
inventories and indexes before copying the catalog into `security/apk/`.
Mutable resolution is confined to this candidate-generation path. The C-2
catalogs were generated in bounded resolution containers in the existing local
validation VM; that is input preparation, not hosted acceptance evidence.

On relevant PRs, `CI / APK Inputs` calls `apk-inputs.yml`, required by
`CI Pipeline`; the same workflow runs weekly. `APK Policy` runs the portable
catalog/caller negatives. Two native `APK Closures (arch)` jobs run all three
closures: successful offline replay (including an unlisted poison archive),
wrong base/architecture, duplicate header, unsafe origin, wrong base inventory,
wrong checksum, corrupt archive, bad/missing signatures with updated catalog
checksums, missing archive/dependency, signed identity mismatch and wrong final
inventory. Pre-install rejections require unchanged installed database bytes;
a wrong catalog final inventory must also reject before changing the live
database. An injected drift after the live install must reject the completed
transaction without issuing a success record. Two `APK Dockerfile Layers (arch)`
jobs build the three real
package-stage prefixes with their physical offline install boundary and inspect
retained inventories. They also inspect **every layer** from `docker save` and
reject archive payloads, including archives deleted in a later layer. Native
closure jobs stage and validate the exact durable-release payload as a
non-publishing rehearsal. Seven-day artifacts contain exact prefixes, logs,
layer results, staged catalog metadata and package records. These jobs have
contents:read only, no repository secrets,
OIDC, registry login or publication. Final application images, SBOMs and browser
behavior require the later packaging jobs; package prefixes do not prove those.

Alpine's public repository removes superseded packages. Catalog schema v2
therefore prefers release assets owned by this repository, under
`build-inputs-apk-<refreshed-on>`, with SHA-256/architecture/name/version asset
names. Every build (PR, main, release and schedule) tries the durable asset
first. **Only HTTP 404** permits fallback to the catalog's original Alpine
HTTPS URL. A prominent `::warning::` names the missing archive release and
upstream URL. Both sources must pass the same catalog checksum, Alpine
signature and signed `.PKGINFO` identity checks before offline installation.
A durable checksum mismatch, 5xx, timeout, TLS failure or unexpected redirect
fails closed without fallback. GitHub's HTTPS release-asset CDN is the sole
external redirect origin admitted; HTTP, credentials and host lookalikes are
rejected. Download headers are checked before accepting bytes or permitting
fallback. A present durable asset never contacts Alpine.

The **APK durable archives present** job separately requires the release and
every current catalog/archive asset, including matching SHA-256 asset metadata
and completed upload state. It runs on PRs with a visible, non-blocking failure
and on the weekly schedule with a failing workflow result. Publication does
not depend on this availability check. Missing archives are loud while fully
verified upstream bytes can keep builds working.

Provisioning is a **post-merge maintainer step that restores durability**;
it is not required for correctness. Once the workflow has landed on main,
use GitHub Actions → **Provision APK Archives** → Run workflow on **main**,
supplying the full reviewed catalog commit SHA in `catalog_commit`. Its two
native read-only preparation jobs execute the trusted workflow revision's
installer against the candidate catalogs, verifying every upstream checksum,
Alpine signature and signed identity. One separate `contents: write` publisher
validates all six catalogs and archive hashes again against the reviewed
commit, then stores a non-latest prerelease. Existing assets must match;
they are never clobbered. No new secret or OIDC identity is needed. The admin
must allow the job's `GITHUB_TOKEN` to create repository releases and upload
assets. The only tag/release it creates is
`build-inputs-apk-YYYY-MM-DD`. Retain these releases for supported catalogs.

The workflow is not invoked by this change. Native PR closure jobs rehearse
payload staging and successful HTTP-404 fallback with real Alpine signature
and signed-identity verification, including tampered signature/identity
rejections. The explicit `provision-upstream` mode is confined to maintainer
payload preparation and logs each upstream source. Weekly candidate refresh produces new reviewable catalogs and their exact
archive release payloads as described below. The initial catalogs still need
the maintainer provisioning step; scheduled refreshes provision new candidates
before opening their review PR.

`Weekly Build Input Refresh` runs each Monday. Its read-only candidate workflow
selects one UTC date, fetches all twelve APT InRelease files from the previous
UTC day's official HTTPS snapshots, fetches the official curl.se bundle and
its matching HTTPS checksum, and resolves APK closures from the exact pinned
bases on native amd64 and arm64 runners. Generated dates, full signed metadata
hashes, inventories and every archive checksum become reviewable inputs.
The native APT matrix authenticates signatures/indexes/packages and exercises
all nine success/tamper cases for four distro roots on both architectures,
using the exact candidate transport CA. APK jobs replay all eighteen cases for
all three closures on each architecture and stage the exact archive payloads.
HTTPS-derived hashes alone never accept a distro candidate.

The same candidate workflow runs as `CI / Build Input Maintenance` on relevant
PRs and is required by `CI Pipeline`. Its PR jobs have contents:read, no secrets
and no OIDC, and never publish. Only the separate main-schedule publisher has
contents:write and pull-requests:write. After every native candidate job passes,
it admits exactly seven distro catalogs plus the CA bundle/pins and the exact
APK payload, creates a catalog-only review branch, stores immutable
`build-inputs-apk-YYYY-MM-DD` prerelease assets, and opens a review PR. Existing
same-date branches/assets must match; replacement and force pushes are refused.
Candidate archive releases may precede catalog review; accepting their catalogs
still requires a reviewed PR and all consuming integrity checks. Application
release tags and image promotion are outside this maintenance workflow.

Repository admins must allow GitHub Actions to create pull requests and the
scheduled job's GITHUB_TOKEN to create archive releases/upload assets. No new
secret is needed. GitHub does not start a new PR CI run for a PR opened by
GITHUB_TOKEN: a maintainer must push a reviewed follow-up commit, or close and
reopen the PR using their own account, to start required PR checks. Scheduled
candidate evidence cannot substitute for those required checks. The maintenance
publisher is implemented but was not invoked during local validation.

`check-build-input-freshness.py` rejects APT/APK catalogs and CA upstream checks
older than fourteen days, future dates, and a Mozilla bundle source timestamp
older than ninety days even if it was recently checked. The weekly freshness
job and scheduled APT/APK image checks fail on stale inputs. Candidate generation
runs independently of the current-input freshness result, so an overdue catalog
can still receive a verified refresh PR. The PR freshness job evaluates the
proposed committed inputs. The Security Scans and Artifact Packaging workflows
require this same gate on PRs and schedules alongside vulnerability scanning of
the actual application runtime images. Package-prefix builds provide separate
input-replay evidence. See [RELEASE_SCANNING.md](RELEASE_SCANNING.md) for scanner
coverage, exception review and currently blocking findings.

Dependabot covers the Cargo/npm roots, Actions, Dockerfile base digests and
hash-locked Python requirements. `renovate.json` adds Monday version proposals
for standalone Rust/npm/bootstrap/SBOM tools, Node, hosted Codex/ACP/Heca,
CUA releases and the CMA source revision. Enable the Renovate GitHub App for
this repository and grant it read access to the private CMA repository to
activate those proposals; no Renovate credential is added here. Automerge is
disabled. Version-only proposals must refresh their official archive URLs,
checksums, dependency locks, pin-resolution records and reviewed output hashes
before acceptance. CUA admission rejects a changed version tag with old archive
identities. Distro/CA weekly updates use the repository-owned workflow above
and do not depend on Renovate activation.

The repository requires these explicit admin/maintainer steps:

| Prerequisite | Action | Until it is complete |
| --- | --- | --- |
| Renovate activation | Enable the Renovate GitHub App for this repository. | Standalone tool, hosted Node/Heca, CUA and CMA revision proposals are absent. Dependabot and the repository-owned APT/APK/CA refresh workflow still operate; committed pin verification stays enforced. |
| Private CMA access | Grant the Renovate App installation read access to `ChronoAIProject/cma`, as well as this repository. | CMA revision lookups and update proposals fail. Existing hosted builds still require the reviewed patch hashes, and public pin proposals remain available. |
| Initial durable APK archives | After merge, dispatch **Provision APK Archives** on **main**, with the full reviewed catalog commit SHA in `catalog_commit`, as detailed above. Retain the resulting archive releases. | The durability job reports missing assets (non-blocking on PRs, failing on schedule). Builds warn and use upstream only after a durable HTTP 404, with all checksum/signature/identity checks intact. Removal of an unprovisioned upstream version can then break builds. |
| Scheduled maintenance publication | Allow GitHub Actions to create pull requests and permit the scheduled job token to create archive releases/upload assets. | Native candidate checks still run, but the publisher cannot complete archive storage or open its refresh PR. Without reviewed manual refreshes, the freshness gates eventually fail. |

The hosted provider dependencies use `security/hosted-tools/package-lock.json`
with exact Codex 0.147.0 and ACP 1.4.0 roots. The initial lock contains 24
SHA-512-bound registry packages, including optional platform binaries, with no
install hooks. The independently buildable `hosted-tools` Dockerfile stage uses
the checksummed npm bootstrap and `npm ci --ignore-scripts`; its committed
`.npmrc` also disables hooks. Offline checks retain the existing shim, native
binary and ACP output identities. Both later stages copy the complete locked
`node_modules` tree so hoisted transitive dependencies reach the shipped image.
Policy rejects global/unlocked installs, hook-bearing dependencies, lifecycle
approvals, missing integrity, manifest/lock drift and incomplete runtime copies.
`Hosted Locked Tools (amd64/arm64)` builds this actual public target on native
PR and scheduled runners, then checks the unchanged lock, installed hook
absence, Codex execution and ACP startup with closed stdin and no network.
These checks require neither private CMA access nor an application compilation;
full hosted-image behavior still needs its private-context build acceptance.

The hosted machine's external CMA patches are bound by
`security/hosted-context.tsv` to a source commit and three ordered SHA-256 values.
`verify-hosted-context.py` verifies every file before any Node execution, rejects
missing/symlinked/changed files and noncanonical names/order, then executes only
private temporary copies of the verified bytes. The read-only build context's
private source is removed before the layer is committed. The existing final
patched ACP bundle hash remains required. Portable PR tests prove ordering and
rejection; they do not claim a full private-context hosted image build.

The reproducibility contract fixes source revision, language tools, dependency
locks, Actions, base image indices, downloaded tool checksums, APT snapshots and
APK archives. **Bit-for-bit image reproducibility is not claimed.** Hosted
runner OS images remain variable. Shipped inventories must still be scanned,
included in actual-artifact SBOMs and bound to final subject digests. A source
lockfile by itself is never the shipped-artifact SBOM. Linux acceptance of the
new APT/APK gates remains pending until their PR CI runs pass.

The proc-macro-error2 2.0.1 vendor directory preserves the authenticated original
crate/license and documents the one-line E0365 visibility correction. The
workspace patch covers all consumers. This is a compiler compatibility fix;
warnings are not suppressed and its removal requires a fixed upstream release.

The **APK durable archives present** PR check emits an explicit warning and the
exact provisioning command when a release or asset is missing. This availability
check exits successfully on PRs so unprovisioned archives do not make required
PR CI red. API/network failures still fail. The same check fails on schedules
(and on a direct release/tag/local invocation) until the release is complete.
It is not a publication correctness prerequisite: every package source still
requires its catalog checksum, Alpine signature and signed identity checks.
After merge, dispatch `provision-apk-archives.yml` on `main` with `catalog_commit`
set to the full reviewed commit containing the current catalogs, then rerun the
availability check. No provisioning is performed by PR jobs.

Machine images are agent workstations. Their declared runtime tool surface
includes `build-essential` and its compiler/header closure for C/C++ builds,
make, npm native installs and pip local wheels, as validated in
`MACHINE_CONTEXT_SPIKE.md`. Package presence alone is not a vulnerability;
HIGH/CRITICAL/UNKNOWN findings still block until fixed or individually reviewed
under the exact policy in `RELEASE_IMAGE_REMEDIATION.md`.
Hosted desktop keeps GitHub CLI for agent workflows, installed
from the checksum-pinned upstream gh 2.102.0 archives for each architecture,
rather than Ubuntu's vulnerable gh 2.45.0. Renovate covers that version pin;
changed release archive hashes require review. The Glama runtime now upgrades
inherited packages through the same signed APT snapshot installer, including
liblzma5. The October 20 catalog is accepted only after native APT replay and
final-image scans in PR CI; local catalog validation is not Linux replay.

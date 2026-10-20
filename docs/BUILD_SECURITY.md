# Build input and shipped artifact security

`security/build-input-pins.json` records the authenticated GitHub commit API
resolution of each third-party Action and the HTTPS Registry v2 resolution of
each Docker base index. The image receipt records its supported platforms; the
SHA-256 of the raw index bytes matched its registry digest. These receipts do
not claim upstream image signatures. Dockerfiles retain readable version tags
alongside the immutable index digests. Dependabot proposes updates; changed
pins also require refreshed resolution receipts and validation.

The repository toolchain is Rust 1.98.1. Pinning dtolnay's Action requires an
explicit `toolchain` input because its full commit no longer communicates a
version tag. Rust builds enforce Cargo.lock, including dependency-cache builds.
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
Catalog URLs must use HTTPS. Only the installer derives HTTP transport URLs;
HTTP transport is safe here only because the reviewed SHA-pinned **signed
InRelease** authenticates those bytes, followed by its authenticated native
indexes and package checksums. Ubuntu redirects HTTP snapshots back to HTTPS,
so minimal images additionally need transport trust before installing CA
packages. Every caller carries `security/apt-transport-ca.pem`, the public
Mozilla bundle extracted by curl as of 2026-09-25, obtained from
`https://curl.se/ca/cacert.pem` and compared with its HTTPS `.sha256` publication.
Its exact SHA-256 is
`a41b5d356aea97a529fe27e0f7316d2f9d946d75927476cf9cf1b90637d00505`.
The installer rejects missing, symlinked or changed bundle bytes before any
APT request. It supplies this file only via `Acquire::https::CaInfo`, requires
peer and hostname verification, and never adds it to the image's system trust
store. Distro `ca-certificates` packages install normally only after the
snapshot metadata, indexes and archives have passed their independent checks.
CI and release-host wrappers use that same checked bundle and installer.
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
payload preparation and logs each upstream source. Weekly candidate refresh
(C-3) must produce new reviewed catalogs and the corresponding release
payloads. Provision them promptly to preserve availability after Alpine
removes superseded packages.

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

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
HTTP bootstraps minimal images without CA packages; distro signatures and the
independent catalog hash authenticate those bytes. Every selected component's
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
actual package-stage prefixes without cache, then inspect records in containers
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
preflights the complete local closure, and installs with APK networking,
repository files and cache disabled. Inventories and receipts are written
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
a wrong final inventory must reject the completed transaction without issuing
a success record. Two `APK Dockerfile Layers (arch)` jobs build the three real
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
therefore consumes release assets owned by this repository, under
`build-inputs-apk-<refreshed-on>`, with SHA-256/architecture/name/version asset
names. The upstream URL is retained for review and candidate preparation.
Normal image builds and weekly replay use **only** the durable URL; neither
fetches package bytes from `dl-cdn.alpinelinux.org`. Changing or corrupting a
release asset still fails its catalog checksum, distro signature or signed
identity. Retain these archive releases for every supported catalog; do not
delete or replace their assets.

A repository maintainer must provision the initial durable release **before
normal image publication or the strict scheduled APK jobs can succeed**. The
workflow is intentionally not invoked by this change. Once this workflow has
landed on main, use GitHub Actions → **Provision APK Archives** → Run workflow
on **main**, supplying the full reviewed catalog commit SHA in `catalog_commit`.
The workflow accepts only a full 40-character SHA and runs only from main. Its
two native read-only preparation jobs execute the trusted workflow revision's
installer against the candidate catalogs, verifying each upstream archive's
checksum, Alpine signature and signed identity before staging assets. One
separate `contents: write` job validates all six catalogs and archive digests
again using trusted code and the reviewed commit's catalog bytes, then uploads
to a prerelease marked non-latest. It never clobbers assets: an existing asset
must match the expected bytes. No new secret, OIDC identity, application tag,
image push or signing operation is needed. The admin must allow the job's
`GITHUB_TOKEN` to create repository releases and upload assets. The workflow
creates only the catalog's `build-inputs-apk-YYYY-MM-DD` archive tag/release.

Before that maintainer step, PR evidence remains possible through the explicit
`pr-upstream-if-missing` rehearsal input. CI supplies it **only for
pull_request**; the reusable workflow checks the event again. Every missing
durable asset must return HTTP 404 before the fetcher logs
`APK PR FALLBACK (durable asset HTTP 404)` and tries the catalog's provenance
URL. No fallback occurs on timeout, TLS/auth/rate-limit/server errors, bad
checksums, signatures or signed identities. All checks remain mandatory and
archives install with no network from read-only mounts. Logs identify the
actual source for every archive. This PR evidence proves input integrity and
packaging while durable-release provisioning remains an explicit outstanding
prerequisite. Production builds, publication CI and schedules do not pass this
input, so absence of durable assets blocks them. The existing Machine Container
E2E job receives explicitly built test/production updater images from its
verified source export, with the same event-scoped PR fallback; its production
publication caller uses durable mode. The separate
`provision-upstream` mode is solely for maintainer payload preparation and
logs each upstream source. After provisioning, both PR and strict jobs consume
the durable release. Weekly candidate refresh automation (C-3) must produce
new reviewed catalogs **and** the corresponding durable release payloads.

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

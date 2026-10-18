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

The reproducibility contract currently fixes source revision, language tools,
dependency locks, Actions, base image indices and explicit downloaded tool
checksums. **Bit-for-bit image reproducibility is not claimed**: Ubuntu/Debian/
Alpine package installation and hosted runner images still resolve signed
distribution repositories at build time. Their exact installed package versions
must be recorded in the shipped image SBOM, scanned and bound to its digest.
Rebuilding an earlier image with the same OS package bytes additionally requires
the retained distribution snapshot or original staged image digest. A source
lockfile by itself is never the shipped-artifact SBOM.

The proc-macro-error2 2.0.1 vendor directory preserves the authenticated original
crate/license and documents the one-line E0365 visibility correction. The
workspace patch covers all consumers. This is a compiler compatibility fix;
warnings are not suppressed and its removal requires a fixed upstream release.

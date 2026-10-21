# Rust artifact dependency metadata and SBOMs

All shipped Rust build definitions use the reviewed `cargo-auditable` 0.7.7
binary from `security/build-tools.json`. The bootstrap verifies its archive
before execution and selects the actual Darwin, GNU Linux, or musl Linux ABI.
Unknown hosts and unavailable per-tool targets fail. Alpine must use the musl
binary; adding glibc compatibility is not a substitute for selecting it.
Cargo-dist's generated installer commands remain prohibited in workflows,
including its `install_cargo_auditable` slot. The release workflows bootstrap
the pinned tool directly.

Cargo-auditable records dependency metadata in the linked application. This
metadata survives the shipped `dist` profile's `strip = true`. Stable Cargo
metadata can conservatively contain extra dependencies; neither the recorded
graph nor its SBOM claims per-crate emitted-byte attribution. Build and runtime
classifications remain explicit. A source Cargo.lock inventory is a separate
artifact, not evidence that a package occurs in a particular binary.

The isolated, locked `security/artifact-tools` Rust root uses the maintained
`auditable-info` parser to inspect ELF, Mach-O, PE, and Wasm without running the
subject. It bounds input to 1 GiB and decompressed metadata to 8 MiB, validates
the graph, and emits the actual binary SHA256/size with its dependency graph.
The root has its own Dependabot, formatting, Clippy, and dependency-scan
coverage. Actual acceptance currently includes stripped application Mach-O
files and a stripped Alpine fixture ELF; other final platform subjects still
require inspection in their publication gate.

`rust-binary-sbom.py` rehashes the final binary, requires its expected application
root and a matching pinned Syft 1.54.1 file inventory, then emits CycloneDX 1.6.
Only dependencies identified as crates.io receive registry cargo purls.
Workspace/local code binds to the specified repository commit. Unsupported git
or other registry origins fail until an authenticated origin mapping exists.
The reviewed local proc-macro-error2 patch has authenticated upstream ancestor
and exact patch/file pedigree in `security/rust-local-provenance.json`. Its
patched component does not claim to be the original crates.io archive. Source
and upstream-ancestor scanning must still assess that dependency/version;
pedigree is not a vulnerability exemption or evidence that every scanner
traverses ancestors.

For the CLI, the source-aware Node wizard seal verifier runs first.
`verify-embedded-wizard.py` then requires every sealed asset and the seal itself
verbatim inside the actual final uncompressed RustEmbed binary after stripping.
Its receipt binds the binary digest, raw seal digest, and each asset's offset.
The CLI SBOM merges retained JS/CSS package identities, static provenance, HTML
attribution, and final asset hashes from that verified seal. This is separate
from Rust dependency metadata and from the archive containing the executable.

Run the following from a clean reviewed checkout. Use a supported Python 3.9+
runner for schema processing; focal's Python 3.8 only runs the compatible build
bootstrap. A global gate can inspect all foreign-platform binaries without
executing them.

```sh
python3 scripts/bootstrap-build-tool.py cargo-auditable --bin-dir "$RUNNER_TEMP/build-tools"
python3 scripts/bootstrap-build-tool.py syft --bin-dir "$RUNNER_TEMP/build-tools"
export PATH="$RUNNER_TEMP/build-tools:$PATH"
cargo auditable build --locked --profile dist -p nyxid-cli --bin nyxid
cargo build --locked --manifest-path security/artifact-tools/Cargo.toml
node frontend/scripts/seal-shipped-composition.mjs wizard verify
python3 scripts/verify-embedded-wizard.py target/dist/nyxid --output embedded-wizard.json
security/artifact-tools/target/debug/nyxid-artifact-info target/dist/nyxid > binary-info.json
syft scan file:target/dist/nyxid -o syft-json > binary.syft.json
python3 scripts/rust-binary-sbom.py target/dist/nyxid \
  --receipt binary-info.json --syft binary.syft.json --package nyxid-cli \
  --commit "$GITHUB_SHA" --wizard-seal cli/src/wizard/assets/shipped-composition.json \
  --embedded-wizard embedded-wizard.json --output binary.cdx.json
python3 -m pip install --require-hashes --only-binary=:all: -r security/python-sbom-requirements.txt
python3 scripts/validate-cyclonedx.py binary.cdx.json
```

An updater uses `--package nyxid-machine-updater` and omits wizard arguments.
Local working builds must record their source diff separately and use the
`--source-diff` digest property; those receipts are investigation evidence,
not clean-release provenance. Detached SBOMs, signatures and bundles stay
outside sealed app/wizard directories. Archive/image digests, complete expected
target coverage, source commit/epoch, scan success, official signing identity,
and image-extracted credential parity must also pass before publication. This
metadata batch does not authorize or establish those later gates.

The official CycloneDX 1.6 tag resolves to commit
`55343ba19dee1785acf1ce9191540d5fd7b590db`. Its schema, referenced SPDX/JSF
schemas and Apache license are vendored unchanged with exact hashes under
`security/schemas/cyclonedx-1.6`. Validation uses integrity-locked Python wheels
and an offline registry; schema tampering or unknown references fail instead
of downloading validation inputs at runtime.

# Dependency advisory remediation proposal

Approved with PM decisions D1/D3, 2026-10-20. C-6 follows completion of these conditions. Steps 1 and 2,
the backend direct-dependency portion of step 3, the scoped argparse override
and its compatibility tests in step 4, and the exact existing-patch dispositions
in step 5 are approved. The PM subsequently chose removal of unused Swagger UI, a build-tooling-only
braces disposition with exported-bundle evidence, immediate AWS-LC bootstrap
migration, and precise public-operation / pinned SSH-signing RSA dispositions.
Earlier alternatives below are retained as proposal history, superseded by
these final decisions; no Swagger fork or braces patch will be implemented. Analysis
used the shared tree at `4fa83f0e9`, the C-5 audit reports, registry metadata and
authenticated candidate source archives. No Cargo build, test or Clippy ran.

Every subsequent local Cargo build/test/Clippy in the shared tree must use:

```sh
CARGO_TARGET_DIR=/tmp/nyxid-shared-target \
CARGO_BUILD_JOBS=4 \
CARGO_INCREMENTAL=0 \
CARGO_PROFILE_DEV_DEBUG=0 \
CARGO_PROFILE_TEST_DEBUG=0 \
cargo <approved command>
```

No extra profile overrides. Wait for Cargo's target lock; never terminate a
different stream's process. Backend consuming tests should run in PR CI.

## Proposed implementation batches

| Batch | Exact candidate change | Approval surface | Acceptance |
| --- | --- | --- | --- |
| 1 | crossbeam-epoch `0.9.18 -> 0.9.21`; h2 `0.4.13 -> 0.4.20`, with precise updates and no unrelated package refresh | `Cargo.lock` | No RUSTSEC-2026-0204 or 0258 in the audit; locked graph/feature checks and existing HTTP/KMS/CLI tests |
| 2 | self_update `0.44 -> 1.3.1`; replace renamed `compression-flate2` feature with `compression-tar-gz`, retaining reqwest/rustls/tar/zip capabilities | `cli/Cargo.toml`, `Cargo.lock` | Old quick-xml 0.38 and self_update's zip 6 disappear; existing update extraction, malformed archive and W6 attestation/trust tests pass |
| 3 | backend zip `6 -> 9.0.5`, keeping default features off and `deflate-flate2-zlib-rs`; retain Swagger UI 9 API through an authenticated local crate patch changing its zip build dependency `3 -> 9`; enable its existing vendored-assets feature | `backend/Cargo.toml`, root `Cargo.toml` patch entry, `Cargo.lock`; reviewed crate patch/source/provenance under `security/vendor/` | No zip below 9; bounded DOCX/skill parsing and ZIP writer regressions; actual backend image and Swagger route/assets work without a mutable UI ZIP download |
| 4 | scoped mobile override `js-yaml@3>argparse: 2.0.1`, removing the only locked sprintf-js path; retain braces 3.0.3 with a new reviewed depth/AST hardening patch because no fixed release exists | `mobile/package.json`, `mobile/pnpm-lock.yaml`, new `mobile/patches/braces@3.0.3.patch`, focused regression additions | Frozen clean install, full audit, legacy YAML CLI/NYC config compatibility, bounded brace parsing/compile/expand/stringify and real micromatch/Metro consumers |
| 5 | exact, expiring patched-package dispositions after executable patch verification; RSA remains a separate decision | Scanner policy/scripts and workflow wiring; reviewed policy records under `security/` | All identity/tamper/expiry negatives; unpatched originals demonstrate the security regression; actual installed patched modules pass it; unrelated findings remain blocking |

The approved Cargo edits ship together in one focused commit, followed by a
locked workspace check with both KMS features under the shared Cargo policy.
Subsequent manifest/lock changes still require per-change PM approval.
Do not combine an unexpected consuming-code migration with a lock update: report
its exact paths and obtain ownership approval first. The implementation status below distinguishes approved changes from held work.

## Rust graph and upgrade limits

- crossbeam-epoch comes through moka -> hickory-resolver -> mongodb. The fixed
  floor is 0.9.20; 0.9.21 is the current compatible release.
- h2 is shared by hyper, both reqwest versions, tonic and AWS Smithy. The fixed
  floor is 0.4.16; 0.4.20 is the current compatible release. Existing users must
  all resolve to the fixed 0.4 release.
- The only quick-xml 0.38.4 parent is self_update 0.44.0. Backend and plist
  already use 0.41.0. self_update 1.3.1 requires quick-xml ^0.41 only for its
  optional S3 backend; our selected features do not enable S3. It requires zip
  ^9 for ZIP extraction. The authenticated 1.3.1 source retains the used
  `Extract::from_source` and `extract_file` APIs. Unix updates already use the
  application's bounded tar reader; preserve the ZIP path and extraction tests.
- zip 3.0.0 comes only from utoipa-swagger-ui 9.0.2; zip 6.0.0 comes from backend
  and self_update. All three zip advisories require >=9.0.0.
- **Swagger UI 10.0.1 is insufficient:** its build dependency is still zip ^8.6,
  and it also moves to utoipa 6. A version-only upgrade would leave the advisory
  open while adding an unrelated OpenAPI migration. The proposed local patch
  keeps the current public API and uses zip 9.0.5. Its original registry archive
  checksum is `d047458f1b5b65237c2f6dc6db136945667f40a7668627b3490b9513a3d43a55`.
- Swagger 9's build script manually copies entries and applies Unix modes; a
  zip dependency upgrade alone does not harden that custom code. The patch must
  reject symlink entries, retain enclosed-path checks, and mask privileged mode
  bits. Enable the existing `vendored` feature so static UI bytes come through
  a checksum-locked crate instead of its mutable GitHub tag ZIP download. Add
  candidate refresh coverage and exact source/patch verification like the
  existing authenticated proc-macro-error2 patch. No scanner suppression for
  the old zip versions is proposed.

Authenticated zip 9.0.5 source retains `central_directory_start`, `by_index`,
`encrypted`, `unix_mode` and `SimpleFileOptions`, used by existing consumers.
Its Rust floor and self_update 1.3.1's floor are 1.88, below the workspace's 1.93
floor and pinned compiler 1.98.1. Resolution and compilation are authorized for the approved dependency changes;
source inspection is not consuming-test evidence.

## RSA / RUSTSEC-2023-0071

The audit covers rsa 0.9.10 and 0.10.0-rc.18 with no patched or unaffected range.
The latest stable is still 0.9.10. A newer 0.10 release candidate does not itself
establish that the advisory is fixed; do not move to it or ignore the advisory
as an automatic scanner fix.

| Path | Observed operation | Implication |
| --- | --- | --- |
| `backend/src/crypto/jwt.rs` | rsa 0.9 key generation for development/startup and public-key parsing/JWKS; request-time JWT signatures use jsonwebtoken's explicitly selected AWS-LC backend | No RustCrypto RSA private decryption at the observed JWT request path. This does not clear the shared dependency finding. |
| `backend/src/services/ssh_service.rs` | Generates and uses an Ed25519 CA through ssh-key 0.6.7; enabled default ssh-key features also retain rsa 0.9 in the lock | Public-key parsing and dependency presence must be distinguished from private RSA operations. Preserve existing supported user-key formats. |
| `machine/src/bootstrap.rs` / `cli/src/node/managed_runtime.rs` | rsa 0.9 RSA-3072 OAEP/SHA-256 wraps the bootstrap AES key. Server `seal` is public-key encryption; client `open` calls `decrypt_blinded(OsRng, Oaep, ...)` after canonical size/binding checks on a configured-server response | This is a real private RSA operation on transported ciphertext. TLS, blinding, OAEP, binding checks and delivery scope reduce exposure but do not prove constant-time behavior or justify a broad exception. |
| `cli/src/node/ssh_node_exec.rs`, russh 0.63.3 and ssh-key 0.7.0-rc.11 | Optional imported RSA client keys sign SSH authentication transcripts; RSA host-key checks are public verification. rsa 0.10 is also used while importing key formats | No observed PKCS#1 v1.5 private decryption oracle. Signing is a distinct private operation requiring a precise review; do not label all CLI RSA use local-only. |

The source search found no application call to RustCrypto PKCS#1 v1.5 private
decryption. This is reachability evidence for the named paths, not a timing proof.
No timing attack, credential probe, RSA feature removal or protocol migration ran.

Recommendation: keep both findings blocking while the crypto owner decides
whether the precisely described uses warrant a time-limited risk acceptance or
must move to reviewed alternative primitives/providers. A provider change must
preserve bootstrap wire/persistence compatibility or receive separate protocol
approval; SSH RSA support must not silently disappear. Any accepted disposition
must bind exact package/version/checksum, lock path/content, selected features,
reviewed call-site evidence, owner, rationale and expiry, and fail on a new
private-operation path or changed inputs. Do not add a global cargo-audit ignore.

## Mobile choices and patch dispositions

braces 3.0.3 is still the latest registry release and is affected by
GHSA-vfj7-8cjw-p6xm. Its path is Expo -> Metro file map -> micromatch -> braces.
No version override currently reaches a fixed release. Keep the exact version
and implement a small reviewed patch covering all public pattern/AST entrypoints,
including compile, expand and stringify, with bounded depth before recursive
walkers and validation of externally supplied ASTs. Reject cyclic/malformed ASTs
and excessive depth with a controlled error before stack exhaustion. Test both
ordinary brace/range semantics and below-size-limit deeply nested inputs through
real micromatch. A patch exception is considered only after those checks pass.

sprintf-js 1.1.3 is also affected by GHSA-hp3w-g68c-fv3c, so upgrading 1.0.3 to
latest would not fix it. The sole audited path is React Native -> babel-jest ->
Istanbul -> load-nyc-config -> js-yaml 3 -> argparse 1 -> sprintf-js. A scoped
argparse 2.0.1 override removes that dependency and preserves the old CLI API.
Authenticated temporary-source probes passed 3/3 cases (help, valid YAML to
JSON, expected malformed-YAML failure). argparse 3.0.3 failed all three because
it rejects legacy constructor options; it is excluded. The frozen pnpm graph
and actual NYC consumer still need verification after approval.

Existing patches remain intact; no rewrite of accepted W7-R1-05/06 is proposed:

| Package/advisory | Required patch SHA-256 |
| --- | --- |
| decode-uri-component 0.2.2 / GHSA-vcc3-ghjq-m6fr | `2c20276b124d7be1b0bc3353a5bf4e1c9f4845b8b0b25e67c8e53abc8485d7e2` |
| node-forge 1.4.0 / GHSA-86w9-cpqp-85rv | `f30d32139499edc78f751a2a3e4e6c5146e915382de608db48ad8ae65096f6f0` |
| image-size 1.2.1 / GHSA-5p2g-fcmc-qvqq and GHSA-w3rx-r6r6-pgpr | `aed80e6dccee17930510649cd38ec398460c70cc050788eedd26e45a4862bb7a` |

For each proposed patched-package disposition, require exact advisory ID,
package/version, normalized `mobile/pnpm-lock.yaml` path and complete lock hash,
upstream package integrity, normalized patch path and exact patch hash, installed
patched-source hashes, owner, rationale and expiry. Require the pnpm patch map
and every resolved snapshot to carry that same patch hash. A changed lock needs
an explicit disposition refresh rather than silently expanding coverage.

In the required PR/scheduled dependency job, use the locked package manager for
a clean frozen install without install hooks, then execute the existing eight
security regressions against those actual installed modules. Separately apply
the committed patch to an integrity-verified original archive and require the
installed reviewed files to match. Verify that the same targeted regressions
fail against the original vulnerable source without disclosing keys. Reuse the
accepted tests; add only the binding/integration negatives that they lack.

Negative cases must include missing/changed/moved patch, wrong advisory or
package version, changed original archive, edited/unpatched installed module,
unpatched duplicate lock snapshot, changed lock content, expired record,
missing regression evidence and a new advisory on the same package. The raw
registry report remains complete; the gate reports an exact patched disposition
only after this verification. No package-wide or advisory-wide audit ignore.

## Evidence and approval boundary

Read-only evidence: the twelve-root C-5 audit, direct lock graph inspection,
registry version/dependency metadata, authenticated self_update/zip source API
inspection, and the three argparse 2 compatibility probes. No shared Cargo
manifest/lock or mobile manifest/lock changed, no shared Cargo build was launched,
and Lima was not used for this planning work.

For the approved steps, run precise lock updates in focused commits and re-run the full
dependency audit. Run small CLI/machine/unit consumers locally only under the
shared Cargo policy. PR evidence must include Every Dependency Root Audit,
Scanner Policy, Mobile, CLI Test, Rust Features/KMS coverage, backend targeted
ZIP/DOCX/Swagger tests, final Artifact Packaging images and archive identities.
Keep C-6 blocked until the PM sends the C-5 verdict independently of this plan.

## Approved Cargo implementation

The focused Cargo commit updates crossbeam-epoch to 0.9.21, h2 to 0.4.20,
self_update to 1.3.1 with `compression-tar-gz`, and backend zip to 9.0.5.
zip 9 requires nix >=0.31.3, which requires libc >=0.2.186; those transitive
updates are necessary. The unused Swagger UI dependency and its zip 3 build dependency are removed.
Managed bootstrap generation, sealing and opening use AWS-LC with unchanged
RSA-3072/OAEP-SHA256/MGF1-SHA256/domain-label and PKCS#8/SPKI wire formats.
Borrowed pkcs1/pkcs8 parsers retain bounded canonical-key validation; rsa is
only a machine test dependency for cross-provider interoperability. Existing
Python fixture, bounds, tamper, label and private-persistence tests remain.
The requested locked workspace check includes `nyxid/aws-kms,nyxid/gcp-kms`.

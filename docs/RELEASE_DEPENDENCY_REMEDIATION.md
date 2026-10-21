# Reviewed dependency remediation

The C-5 dependency gate retains the complete upstream audit reports. It blocks
all unresolved findings; an exact reviewed disposition requires fresh graph,
source and executable evidence in the same job. There are no cargo-audit ignores,
pnpm audit ignores or package-wide suppressions.

## Cargo upgrades and bootstrap crypto

The focused Cargo change updates crossbeam-epoch to 0.9.21, h2 to 0.4.20,
self_update to 1.3.1 and backend zip to 9.0.5. self_update keeps tar, zip and
Rustls TLS support with the renamed `compression-tar-gz` feature. zip 9 requires
nix >=0.31.3 and libc >=0.2.186. The unused `utoipa-swagger-ui` dependency is
removed, eliminating zip 3 and its mutable build-time GitHub asset download.
The OpenAPI derives and JSON route remain provided by `utoipa`.

The October 21 bundle audit also found the newly published emSign distrust
advisories RUSTSEC-2026-0381, RUSTSEC-2026-0382 and RUSTSEC-2026-0383. The lock
now uses `rustls-webpki` 0.103.16, `webpki-roots` 1.0.10 and
`webpki-root-certs` 1.0.10, which reject the distrusted intermediates or remove
the affected root. No disposition was added for these findings. The 0.26.11
webpki-roots compatibility wrapper resolves to the patched 1.0.10 store.

Managed bootstrap RSA generation, sealing and opening now use AWS-LC. The wire
contract remains RSA-3072, OAEP-SHA256 with MGF1-SHA256 and the existing domain
label, canonical PKCS#8 private persistence and SPKI public encoding. Borrowed
pkcs1/pkcs8 parsers preserve canonical-key, exponent, size and binding checks.
Private DER, AES keys and plaintext use `Zeroizing`; AWS-LC zeroizes its owned
DER buffers. RustCrypto rsa is a machine dev dependency solely for malformed-key
and cross-provider tests. Tests cover a RustCrypto server-sealed ciphertext
opened by AWS-LC, the reverse direction, existing Python fixtures, tampering,
wrong label, wrong wrapped-key length, bounds and private persistence.

The locked workspace check includes `nyxid/aws-kms,nyxid/gcp-kms`. Local Cargo
checks follow the PM's shared-target resource policy; PR CI supplies Linux,
image and cross-platform consumer evidence.

## Remaining RSA use decisions

`security/dependency-dispositions.json` binds RUSTSEC-2023-0071 to both exact
remaining rsa versions, registry checksums, affected package entries and reverse parent subgraphs, selected
Linux/KMS features, resolved workspace parent chains and reviewed source hashes.
The records expire on 2026-11-19, matching the bundle's dependency review deadline.

- rsa 0.9.10 remains through the backend's direct dependency and ssh-key 0.6.7,
  plus machine test-only interoperability. Backend uses are public parsing/JWKS
  and local development key generation. Request-time JWT signing uses AWS-LC;
  the SSH CA uses Ed25519. The machine production bootstrap has no RustCrypto
  private operation.
- rsa 0.10.0-rc.18 remains through russh 0.63.3 and ssh-key 0.7.0-rc.11 in the
  CLI. RSA client signing authenticates to the owner-selected host only after
  its pinned fingerprint is verified. Only that host observes timing.
  `docs/SSH_NODE_KEY_AUTH.md` recommends Ed25519.

The gate compares deterministic `cargo metadata --locked` results against the
reviewed chains and features. A conservative source change guard binds every
RSA/SSH-reference file and counts each private-operation line throughout
workspace source. New imports, aliases, private-operation spellings, duplicate
operation lines or changed reviewed contexts fail and require explicit source
review. This is a change guard, not a claim to infer arbitrary Rust data flow.
Unrelated crypto changes may also require review. Never regenerate the baseline
to accept a new RSA private operation without security review.
The baseline binds committed source. In a shared checkout, another stream's
unfinished edits cannot become part of a risk decision; verify the publication
commit in a clean audit fixture and review any later source changes before
refreshing the baseline.

### Refreshing the RSA source review guard

Run `python3 scripts/dependency_dispositions.py refresh-rsa-guard --root . --dry-run`
from the reviewed branch. It resolves committed `HEAD`, reads its immutable Git
blobs, and prints every changed path and before/after hash or counted line hash.
Dirty files, staged changes and untracked files cannot enter the new baseline.
Review the corresponding `git show HEAD:<path>` bytes and the committed diff
before running the same command without `--dry-run`. The command rewrites only
`rsa_guard`; it cannot renew expiry, change graph decisions or authorize another
advisory. Commit that policy change with the reviewed source revision in the
review report. A moving HEAD aborts the refresh.

Production RSA-reference files retain full-file context hashes. The test-only
`backend/src/test_utils.rs` instead binds every RSA-reference line and its count;
its unrelated Mongo cleanup may change without renewing a crypto decision.
Private-operation lines remain counted in every file, including that fixture.
A new RSA reference, changed or duplicated private operation, changed production
RSA context or new source file still fails admission. This remains a conservative
change detector; reviewers must inspect types and call paths before refreshing.

The October 21 bundle refresh binds committed `c6b4c1837` after merging main's
W1–W4 work. It removes excluded W8/W9.6 paths and their operation counts, and
reviews main's contextual AES-GCM adapters, JWT key import/generation module,
SSH CA context adapter and sanitized SigV4 call. JWT private signing remains
AWS-LC; the SSH CA remains Ed25519; SigV4 is HMAC-SHA256. No new RustCrypto RSA
private signing/decryption operation is admitted. The affected dependency
subgraphs, checksums and selected Linux/KMS features are unchanged. The backend
manifest binding includes the zeroization features and test dependency merged
from main. The full source and finding delta is recorded with bundle validation.

## Mobile YAML compatibility

The scoped `js-yaml@3>argparse: 2.0.1` override removes argparse 1 and sprintf-js
without changing unrelated argparse consumers. Five executable probes resolve
the real React Native/Babel/Istanbul consumer chain, require argparse 2 and no
sprintf-js, exercise js-yaml CLI help, conversion and malformed input, and load
an actual NYC YAML configuration. They run in Mobile CI and the clean dependency
verification install.

## Verified existing mobile patches

`security/dependency-patches.json` records exact reviewed source dispositions:

| Package | Advisory | Patch SHA-256 |
| --- | --- | --- |
| decode-uri-component 0.2.2 | GHSA-vcc3-ghjq-m6fr | `2c20276b124d7be1b0bc3353a5bf4e1c9f4845b8b0b25e67c8e53abc8485d7e2` |
| image-size 1.2.1 | GHSA-5p2g-fcmc-qvqq; GHSA-w3rx-r6r6-pgpr | `aed80e6dccee17930510649cd38ec398460c70cc050788eedd26e45a4862bb7a` |
| node-forge 1.4.0 | GHSA-86w9-cpqp-85rv | `f30d32139499edc78f751a2a3e4e6c5146e915382de608db48ad8ae65096f6f0` |

Each record binds advisory, package/version, normalized lock and patch paths,
full lock and patch hashes, original npm archive sha512 integrity, complete
installed source inventory, regression and compatibility script hashes, owner,
rationale and monthly expiry. The reviewed expiry is 2026-11-19. Patch maps,
resolved snapshots and every dependency edge must name the reviewed patch hash.

The required PR/scheduled audit uses the locked pnpm version for a clean frozen
install with lifecycle hooks and pnpmfile disabled. It executes all eight
existing security regressions against the actual installed packages, applies
each committed patch independently to an integrity-verified original archive,
and compares every source file. The targeted regressions must also demonstrate
the vulnerability in each original archive; missing modules and infrastructure
errors do not count. All bindings and installed sources are checked again after
testing. No externally supplied receipt can grant a disposition.

Negative cases cover missing/changed/moved/symlinked patch inputs, wrong advisory
or version, changed archive, edited/unpatched/extra installed source, duplicate
unpatched snapshots, unpatched dependency edges, changed lock, missing proof,
new same-package advisories and expired or overlong records. Raw audit reports
remain complete, including these upstream-version findings.

## Build-only braces decision

GHSA-vfj7-8cjw-p6xm has an exact monthly `build-tooling-only` record for braces
3.0.3 in `mobile/pnpm-lock.yaml`, bound to its affected package entries and parent subgraph, original archive
integrity and micromatch 4.0.8 parents in Metro file-map and Jest. There is no
braces hardening patch. Repository-authored build configuration supplies the
patterns processed by these tools.

Both Mobile CI and the dependency audit export the actual iOS production app
with source maps and inspect every emitted map and paired bundle. The proof
requires application and React Native modules and rejects braces/micromatch
module paths, including pnpm, Windows and indexed-map forms. The audit exports
from its clean frozen install. A changed parent, runtime inclusion, missing or
failed export, changed checker or lock, expired record or newly published braces
release prevents the disposition.

## Required PR evidence

Review Scanner Policy, Every Dependency Root Audit, Full History Secret Scan
and Mobile. CLI Test, Rust Features/KMS checks and Artifact Packaging verify the
consumers and final images. Local script/unit evidence does not establish that
Linux image builds or final-image vulnerability scans passed. C-6 may proceed
under the PM's conditional C-5 acceptance once these conditions are implemented.

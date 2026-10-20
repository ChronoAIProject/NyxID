# Release security scans

CI Pipeline requires Security Scans on every pull request. The same reusable
workflow runs weekly. It needs read-only repository access and no repository
secret or OIDC permission. Scanner/tool archives have reviewed checksums in
`security/build-tools.json` and weekly Renovate update coverage.

`Full History Secret Scan` fetches complete history, scans all reachable refs,
and independently scans every committed file, including unchanged files.
Inline `gitleaks:allow`, `.gitleaksignore`, inherited scanner environment and
baselines cannot suppress discovery. Symlinks are scanned as their literal
target text, and an unaccounted submodule fails the gate. Raw scanner findings
stay in a private temporary directory and are removed on exit. Logs and the
uploaded report contain locations, content hashes and dispositions without
credential bytes.

`Every Dependency Root Audit` discovers every tracked Cargo, npm and pnpm lock
and accounts for every package/workspace manifest. This includes backend, CLI,
machine and updater in the main Cargo workspace; the independent machine spike
and artifact-tools Cargo locks; frontend, SDK and mobile; hosted/npm tool locks;
the independent CLI test, Claude, OpenClaw and oracle worker npm roots. Missing
locks, orphan locks and unaccounted manifests fail. Raw audits include development dependencies. Mobile also receives a clean
frozen install without lifecycle hooks for executable patch and production-bundle
verification. Isolated directories
and configuration prevent local audit ignores or npm configuration from hiding
findings. Every reported dependency severity blocks this gate.

Artifact Packaging builds and scans actual final images on native amd64 and
arm64 runners on relevant PRs and weekly: backend, frontend, node-agent, machine,
updater, machine-node-only, hosted desktop and mcp-demo. The private hosted final
image remains an operator build; its mandatory wrapper applies the same final
image scan before tagging. No private CMA bytes enter public PR jobs.

Trivy receives the exact Docker archive saved from an immutable image ID. The
scan must identify distribution packages and the actual embedded Rust executable
dependency inventory where applicable, including the scratch updater. HIGH,
CRITICAL and UNKNOWN severities, an unsupported distribution, missing inventory,
scanner errors and malformed reports fail. The full report retains lower
severities and unfixed vulnerabilities. No Trivy ignore file or VEX suppression
is loaded. APT, APK and transport-CA freshness gates accompany scheduled scans.

Image publication waits for source scans before any registry write. It builds
locally, verifies final source identity, scans those exact bytes, validates an OCI
layout and copies only the passing subject by digest. A changed report/archive,
wrong config/platform/source, missing receipt or failed scan prevents the copy.
Attestation and promotion consume that same published digest. CLI release
publication also requires the source history/dependency scan workflow. Mandatory
official release signing is a separate release-integrity gate.

## Exact secret dispositions

`security/secret-exceptions.json` is the complete reviewable exception inventory.
Each entry requires the detector rule, exact normalized relative path, hashes of
the exact detector match, extracted value and full original source lines, plus
owner, expiry, rationale and source provenance. Every field must match. Expired
entries fail globally even when their finding is absent from the current tree.
There are no path-wide, detector-wide or secret-hash-only dispositions.

The Firebase Android client configuration has the separately reviewed public
client role. Its exact exception does not cover another key in the same file.
Real-detector regressions require rejection for a new same-file key, changed
value/content, moved path, wrong rule, expired entry and unrelated documentation
secret. The tests also commit and delete a synthetic secret on another branch
and require full-history detection without printing its bytes. No Firebase key
has been used or probed.

The seven current documentation values use the PM's role-specific placeholders.
An API-key display prefix, newly issued key, rotated key, downstream credential,
provider key and delivered refresh token retain their distinct meanings.
Response examples are illustrative; angle-bracket values are placeholders.
Substitute only values obtained from your own authorized NyxID server or
provider. A display prefix is not a full key, and the delivered refresh token is
distinct from the API key and node identifier.
Replacing current examples does not establish provenance for historical bytes.

## Resolved historical findings

PM review resolved all six value identities below without testing any live
credential. Eight exact finding identities cover ten history findings; each
record binds rule, normalized path, exact match/value/line content and immutable
introducing commits. `history_commits` makes annual entries unusable for the
current tree or a newly introduced commit, even if copied bytes are identical.
Current-tree entries expire monthly (at most 31 days); immutable history-only
entries expire after 12 months (at most 366 days). Any expired entry fails the
global gate, even if a scanner does not currently report it.

| Value SHA-256 prefix | Historical path / introducing revisions | Reviewed disposition and basis |
| --- | --- | --- |
| `a9d3c990476cf66f` | Device-binding doc and devices skill; `b73382431229`, `80b347be7c04` | `inert-historical-credential`: no redemption lookup; only hash row TTL-expires 24 h after approval; introducing doc says the feature had not shipped with production data. Provenance remains unestablished. |
| `4e36e93944591d2a` | Streamline proposal / API doc; `36652be933ec`, `b8ecbd878dc8` | `synthetic-document-example`: sequential hex structure. |
| `a2c62cb92a6c0cd3` | API doc; `a68fbc09c44c` | `synthetic-document-example`: sequential hex structure. |
| `d134b5cf5427c235` | API doc; `67f1d004ec31`, `a48180a2db19` | `synthetic-document-example`: sequential rotated-key groups. |
| `84033187c49cc376` | API doc; `67f1d004ec31` | `synthetic-document-example`: 14-character display prefix, shorter than a usable issued key. |
| `823e6e6619dd8f27` | API doc; `67f1d004ec31` | `synthetic-document-example`: sequential newly issued-key example. |

The inert kind is restricted to the one reviewed value identity, detector and
two documentation paths, with mandatory exact code-evidence fields. That evidence
records `device_codes.refresh_token_hash` writes at `approve.rs:118/165`, the
absence of a redemption lookup, the 24-hour TTL, and the unshipped-feature
statement. It authorizes no other credential class. No token-format or revocation
machinery is added; no endpoint currently redeems this device refresh token.
For a new historical finding, review present redeemability, backing-record
retention and provenance. A redeemable value of unverified provenance requires
the user's rotation decision before any exception is written.

Two additional generic-detector findings are ordinary slash-separated progress
prose in the stream A worker log and the historical remediation checkpoint
table. Their exact rule, path, match, value and source-line hashes are bound
in the same inventory. The current worker-log record expires monthly; the
checkpoint record applies only to revision `61781e414d5c` and expires after
12 months. Neither record grants an exception to any credential value.

The W8 inventory's slash-separated `undeclared/missing/mixed/stale` writer
matrix prose is another generic-detector false positive after the word `key`.
One exact monthly record binds the identical full source line in its introducing
revision `a71e937c1cb9` and the current tree. A real-detector regression adds a
key to that same line and requires both changed findings to remain unresolved.
The W8 document is scanned in full; it has no path-level exemption.

The migration/cleanup mock-keychain unit tests introduced in `f3d78472fe13`
seed the same sequential 16-byte hex fixture at `cli/src/node/agent.rs` lines
4140, 4195 and 4594 of that revision. One monthly `synthetic-test-fixture`
record binds their identical detector rule, normalized path, match, value and
full source-line hashes. The value exists only in mock unit tests alongside
explicit test authentication/credential strings. A real-detector test requires
a changed value, moved path, or new key on the same line to remain unresolved.

## Exact dependency dispositions

The initial twelve-root audit found twelve Cargo advisory occurrences and six
mobile advisory occurrences. The approved upgrades and Swagger UI removal
eliminate crossbeam-epoch, h2, quick-xml and zip findings; the scoped
js-yaml/argparse override removes sprintf-js. Registry reports remain unchanged
by dispositions and are retained with raw and unresolved counts.

`security/dependency-patches.json` binds the four upstream findings on the three
already patched mobile packages to exact advisory/package/version, normalized
lock and patch paths, affected-package/consumer-subgraph and patch/source/regression hashes, original npm
archive integrity, owner, rationale and monthly expiry. A clean frozen install,
complete installed-source comparison, eight existing regressions and failing
vulnerable-original probes must all pass in the same audit process. New
advisories or unreviewed duplicate/unpatched sources remain blocking.

Lock bindings hash the canonical reverse dependency subgraph rooted at each
affected package: exact registry names, versions, archive checksums/integrities,
and every consumer edge through the lock's roots. Workspace release versions
and unrelated lock entries/formatting are excluded. Changing an affected archive,
version or parent chain requires a new review; a new advisory never inherits a
disposition. Cargo's selected feature/normal/dev chains are also checked through
locked metadata. Full before/after lock hashes still detect mutation during
installation, bundle export and audits, without binding an exception to unrelated
dependency updates.

RSA manifest bindings cover the complete canonical parsed TOML, excluding only
the application's package release version (or retaining its workspace-version
inheritance). They include crypto provider selection, dependency definitions,
features and all other manifest values. A v0.73.0 release-number bump cannot
invalidate the v0.72.0 use review; switching JWT providers or RSA dependency
configuration cannot inherit that review. Rust source bindings remain exact
file bytes, with no version normalization.

`security/dependency-dispositions.json` contains the PM's separate use decisions.
The monthly braces 3.0.3 disposition requires exact Metro/Jest parent chains,
lock/archive identity and a real production app export proving no braces or
micromatch module ships. A new braces release forces review. The two RSA
RUSTSEC-2023-0071 records expire within 90 days and bind exact registry checksums,
resolved parent chains/features and reviewed source. Machine bootstrap private
operations moved to AWS-LC. Remaining backend public/generation and pinned
owner-selected SSH client-signing uses are described in
[the remediation record](RELEASE_DEPENDENCY_REMEDIATION.md). A new RSA reference,
private-operation spelling or changed reviewed context fails the source guard;
review must establish the use before updating that baseline.

The release follow-up reviewed four added node-storage AES-GCM call lines in
the committed integration: `managed_runtime.rs` reads an old fixture through
`LocalEncryption`, `oauth/storage.rs` uses `Aes256Gcm` directly, and
`secret_operation.rs` / `secret_transaction.rs` validate fields through
`LocalEncryption` (which uses `Aes256Gcm`). Only their exact line hashes/counts
were added to the conservative private-operation guard. No RSA/SSH reference
file, parent chain, feature, advisory scope or expiry changed. Guard failures
name each changed path and category without printing source or secret bytes.

Every expired record fails globally, including unused entries. Failed or missing
proof never becomes a warning. Scanner Policy tests tamper, scope, new-use and
expiry negatives; Every Dependency Root Audit runs the actual proofs on PRs and
schedules. Mobile independently runs the production export and YAML consumer
compatibility probes.

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
locks, orphan locks and unaccounted manifests fail. Audits run without installing
dependencies or hooks and include development dependencies. Isolated directories
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

## Findings still requiring disposition

The local full-history review identified the following unresolved historical
value identities. Hash prefixes identify findings without exposing values.
All remain blocking; no exception has been written for these values. No live
credential has been confirmed or tested. If owner review establishes that a real
credential appeared, stop exception work and obtain the user's rotation decision.

| Value SHA-256 prefix | Historical path and introducing revision | Disposition |
| --- | --- | --- |
| `a9d3c990476cf66f` | `docs/device-code-binding.md`, `skills/nyxid/references/devices.md`; `b73382431229`, later example at `80b347be7c04` | Three historical findings; refresh-token provenance unverified. |
| `4e36e93944591d2a` | `docs/STREAMLINE_SERVICES_PROPOSAL.md` at `36652be933ec`; `docs/API.md` at `b8ecbd878dc8` | Two historical findings; provider-key provenance unverified. |
| `a2c62cb92a6c0cd3` | `docs/API.md` at `a68fbc09c44c` | One historical finding; downstream-credential provenance unverified. |
| `d134b5cf5427c235` | `docs/API.md` at `67f1d004ec31` and `a48180a2db19` | Two historical findings; rotated-key provenance unverified. |
| `84033187c49cc376` | `docs/API.md` at `67f1d004ec31` | One historical finding; display-prefix provenance unverified. |
| `823e6e6619dd8f27` | `docs/API.md` at `67f1d004ec31` | One historical finding; newly issued-key provenance unverified. |

The first isolated local dependency audit covered twelve roots: ten clean, the
main Cargo lock with twelve advisory occurrences, and mobile with six. The main
lock contains crossbeam-epoch/RUSTSEC-2026-0204, h2/RUSTSEC-2026-0258,
quick-xml/RUSTSEC-2026-0194 and 0195 through self_update, zip/RUSTSEC-2026-0367,
0368 and 0369 through Swagger/self_update/backend, and both RSA versions under
RUSTSEC-2023-0071. RSA has no published fixed version in that audit database.
Dependency changes need separate manifest/lock review and consuming tests.

Mobile reports GHSA-vcc3-ghjq-m6fr, GHSA-5p2g-fcmc-qvqq,
GHSA-w3rx-r6r6-pgpr and GHSA-86w9-cpqp-85rv for the three packages with existing
reviewed source patches. Registry audit still reports their upstream versions;
there is no blanket exception for patched packages. The existing eight patch
regressions and exact patch/lock binding must support any future narrow
disposition. GHSA-vfj7-8cjw-p6xm (braces) and GHSA-hp3w-g68c-fv3c (sprintf-js)
are additional unresolved findings. These results are discovery evidence; they
do not establish a clean dependency or final-image scan.

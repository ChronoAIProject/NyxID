# W5–W9 implementation worker log

> Bundle note: this is the W5, W6, W7 and W9.1–W9.5 subset of this log at
> `e68e4276e`. W8 and W9.6 entries remain with draft PR #1944. Entry statuses
> are historical; current bundle verification is recorded in its PR.

This file records worker changes and executed evidence. It does not replace the
PM-owned execution/review ledgers, and a worker test pass is not a PM review
verdict. Baseline: `f96cf877834445d4976f62691aa192e534535c1f`, branch
`security-review-remediation-report`. The source review report is preserved.

## Runtime and authorization

The actual initial Codex turn context recorded `gpt-6.1-sol`, `xhigh`,
`danger-full-access`, approval `never`, provider `heca_gateway`; PM independently
verified it and authorized implementation. No other workers or reviewer agents
were spawned. Work is local; no publication, deployment, push or merge is part of
this authorization.

Required `CLAUDE.md`, the full source assessment and execution/review documents
were read before coding. Relevant subsystem documents are read before changes.
PM owns checklist/review status and reviews each bounded commit personally.

## W6 verifier and explicit source updates — host verified, PM container review pending

Implemented SET-authenticated Rekor time, exact DSSE body/certificate/signature
binding, Merkle proof and signed checkpoint verification, historical CT/Rekor
key validity and Fulcio trust/anchor validity at event time, GitHub OIDC issuer
and exact workflow/ref identity, bounded JSON/material inputs and restricted
bundle redirects. Production TUF authentication remains mandatory.

Public logged NyxID v0.15.0, v0.40.0 and v0.71.0 producer fixtures are retained
with sources/digests in `cli/tests/fixtures/sigstore/README.md`. All original
fixtures are v0.3, `dsse/0.0.1`; older promise-container policy cases are explicit
transformations. Rekor v2/RFC3161-only bundles remain unsupported and fail closed.

Source updates now require explicit `--from-source`, resolve a real release to a
full immutable commit, use Cargo `--rev` and `--locked`, validate the compiled
version, activate atomically, and persist a source marker/update hold. Missing
releases/assets cannot install default-branch HEAD. The verification-bypass flag
is removed; source builds cannot replace the trusted scheduled controller.

Executed intermediate evidence (superseded by final checks below): initial full
build completed in6m24s, with18 passes/7 fixture failures/1 ignored. Those failures
identified the unsupported assumption that every TUF log ID is SHA256(SPKI):
Rekor v2 uses a note-key ID. Root parsing now retains TUF's format-specific IDs,
and supported Rekor v1 explicitly checks its own ID contract. Subsequent updater
runs passed64 and68 tests with no failures, each leaving only the explicitly
ignored live TUF check. Expanded cryptographic cases assert relevant error
contexts and include independent recursive RFC6962 paths for sizes1..33.

Validation root:
`/var/folders/y2/9pj294vd0qjfp88k419b5j8r0000gn/T/nyxid-w5w9-validation-3moeqwm0`.
Cargo uses its `target` directory, jobs2, incremental off, dev/test debug info0.
Final host commands (same Cargo environment above):

- `cargo test --locked -p nyxid-cli --bin nyxid update -- --nocapture`:
  169 passed, 0 failed, 4 ignored, 1445 filtered (`w6-update-final.log`).
- `cargo test --locked -p nyxid-machine-updater -- --nocapture`: library
  120 passed, 0 failed, 1 ignored; binary 78 passed, 0 failed, 4 ignored;
  doctests 0 (`w6-machine-final.log`). These share verifier source; the counts
  describe separate consumers and are not unique test counts.
- `cargo test --locked -p nyxid-cli --bin nyxid github_tuf_mirror_verifies_root_rotation_expiry_and_target_hash -- --ignored --nocapture`:
  1 passed, 0 failed (`w6-live-tuf-final.log`). This actually refreshed the
  GitHub mirror and checked the authenticated target using persisted state.
- `cargo clippy --locked -p nyxid-machine-updater --all-targets -- -D warnings`:
  passed after factoring the validity-interval return type into an alias
  (`w6-updater-clippy-final.log`).
- `cargo check --locked -p nyxid --bin nyxid-server`: passed
  (`w6-backend-check-final.log`).
- `cargo fmt --all -- --check` and `git diff --check`: passed.

Ignored-test accounting: the live TUF test was explicitly executed above;
`companion_process` is a child-process entrypoint, not standalone acceptance.
`real_container_migration_signed_upgrade_and_rollback` and
`real_companion_handoff_and_rollback` require the Docker harness
`cli/tests/machine_updater_e2e.sh`; PM owns their execution on this stable source
against the immutable released machine supervisor. They remain pending and are
not counted as passes.

The signed mini-TUF tests exercise restart/concurrent process locking, signed
still-unexpired timestamp/snapshot rollback, expiry, legitimate role rotation,
embedded bootstrap advancement ahead/behind cached lineage, incompatible forks,
corrupt/missing initialized state, authenticated legacy cache migration and a
nonroot read-only installation with a writable private state volume. Native,
companion and server state paths are absolute/private/durable, with no ephemeral
fallback. The server registry token is bounded and zeroizing.

Compiler future-compatibility warning: `proc-macro-error2` 2.0.1 reexports a
private `proc_macro` extern crate (E0365, rust-lang/rust#127909). This is recorded
for W7 dependency remediation, not hidden and not counted as a W6 failure.
`w7-future-incompat-inventory.log` retains Cargo's report.

## W5 production browser delivery — host evidence, PM image validation pending

Implemented a validated origin-only nginx renderer and enforcing SPA CSP with
no script inline/eval permission. Complete per-location headers preserve the
backend API/OAuth/standalone policies, including backend paths ending `.js`.
Trusted deployment HTTPS alone enables bare one-year HSTS; Host/forwarded proto
cannot enable it. The backend wrapper now encloses early limiter/router errors.
All SPA delivery uses no-referrer; actual literal/encoded/duplicate query names,
encoded/trailing receiver paths and same-origin resource/external referrers are
covered before JavaScript runs. Access logs omit queries and Referer values.

Backend packaging now includes the standalone credential HTML/scripts/manifest;
frontend and backend credential builders share Node22.23.2, npm lockfile and
source/epoch inputs. Actual shipped-image parity remains a required W7 release
gate. Runtime API/telemetry URLs are validated and custom connection origins
require explicit deployment admission. Bundled PostHog disables remote config,
lazy dependency scripts, flags/surveys/replay while preserving consent/DNT and
redaction. Font/theme inline handlers were removed; Unicorn has pinned SRI.

The real production browser exposed two compatibility failures: Zod compiled
schemas using eval, and MFA did not refresh the live auth store before protected
navigation. Exact build aliases initialize Zod's interpreter before every schema
in SPA/standalone/wizard entry graphs. MFA uses the existing session check, with
PM's one-shot correction: successful/uncertain POSTs are never replayed during
session-confirm retry; 503 preserves confirmation, 401 returns to login for a
fresh challenge, and network uncertainty checks the cookie first. Backend MFA
policy is unchanged. Telegram's vendor data-onauth eval callback was replaced
by bounded schema-validated messages bound to exact origin AND owned frame,
with duplicate/in-flight protection and listener cleanup.

Committed regression harness sources are documented in
`docs/BROWSER_SECURITY_POLICY.md`. Recorded public vendor/font fixtures retain
source/length/SHA256 receipts; cached offline copy was also verified. The harness
serves actual built dist through generated nginx, mocks transport only, collects
real CSP violation events and asserts expected rejection destinations. The
credential transport fixture does not execute the Rust handler; its tests are
separate. Native nginx1.31.6 results do not substitute for production1.27.

Final host evidence uses the validation root above:

- `SOURCE_COMMIT=ec07a6bb SOURCE_DATE_EPOCH=1792358680 npm run build` from frontend:
  passed (`w5-frontend-build-final.log`).
- `npm test -- src/hooks/use-auth.test.tsx src/components/auth/mfa-verify-form.test.tsx src/stores/auth-store.test.ts src/lib/telemetry.test.ts src/schemas/runtime-config.test.ts`:
  5 files,85 tests passed (`w5-vitest-final.log`). The runtime/telemetry cases
  reject missing authority and raw empty query/fragment delimiters; admitted
  origins are canonicalized before API path concatenation.
- `python3 frontend/scripts/test-nginx-policy.py`:4 passed, including origin/policy
  injection and split-origin/response-specific policy checks.
- `verify-production-policy.py` against plain and configured-HTTPS servers:
  22 HTTP responses each passed (`w5-http-boundary-final.json`,
  `w5-https-boundary-final.json`); includes real nginx502/413, nested reply
  2MiB success/33MiB rejection, legal304/405 and header uniqueness.
  The loopback hop is HTTP behind a configured HTTPS boundary, not a TLS test.
- Targeted ESLint, Python compile, Rust fmt and diff whitespace checks passed.

The final browser command is `npm exec playwright test -- --config
playwright.production.config.ts`, with `NYXID_PRODUCTION_TEST_URL`, actual
`NYXID_NGINX_ACCESS_LOG`, recorded `NYXID_TEST_FONT`, Chrome executable and
external JSON output as documented:22 passed,0 failed in30.6s
(`w5-native-browser-final.log`, `w5-native-browser-results.json`). Every test
rejects unexpected CSP violations; the injection case annotates only its three
intended directive/destination pairs. Earlier16/16 browser evidence was
expanded for review. First-failure traces remain outside source; the latest
runtime-negative initial failure was a test locator expecting an absent alert
role, corrected to the actual visible error while preserving CSP rejection.

Final backend command, with the bounded Cargo environment above, was
`cargo test --locked -p nyxid --bin nyxid-server mw::security_headers -- --nocapture`:
23 passed,0 failed,0 ignored (`w5-backend-headers-final.log`), including outermost
429/404 handling. The resulting final-source test binary
`target/debug/deps/nyxid_server-38b69207024186fc handlers::credential_accept --nocapture`
then passed4,0 failed,0 ignored (`w5-backend-credential-final.log`). These tests
exercise local HTTP/handler policy and do not claim database integration.
The host linker reported its large unwind-table performance warning; the known
proc-macro-error2 future-incompatibility remains assigned to W7.

`SOURCE_COMMIT=ec07a6bb SOURCE_DATE_EPOCH=1792358680 npm run build:wizard` passed
and regenerated the embedded wizard assets. The unchanged Rust wizard freshness
test, compiled standalone against existing sha2/hex dependencies to avoid a
second full CLI rebuild, passed1/0 (`w5-wizard-freshness-final.log`).

PM owns exact frontend image1.27/browser, default-entrypoint and invalid-env
execution on this stable source. `smoke-frontend-image.py` runs the packaged
default entrypoint without overrides; image-server mode verifies extracted
dist byte parity and records actual image metadata/headers/logs. Those container
checks are pending, never counted as native passes.

PM additionally reported final W6 ec07a6bb real container acceptance: both
migration/companion rollback parent harness tests passed and production scratch
8 checks passed; PM's original logs/ledger own that independent evidence.

## W7 first bounded batch — pins, bootstraps and compiler compatibility

Resolved all24 existing Action repository/ref pairs through authenticated
GitHub commit APIs; each workflow now uses the full commit. Preserved separate
attestation v1/v3 producer versions. Resolved10 Docker base indices over HTTPS
Registry v2 and compared their raw-byte SHA-256 to the registry digest; receipts
include platforms and source URLs. Production, hosted/node-only, updater, spike
and Glama roots are covered. Rust1.98.1 is explicit even when its Action uses a
SHA. Docker and workflow Rust builds enforce the lockfile. Dependabot covers
Action/base and independent integration directories.

The release/cache workflows no longer execute generated installer/package shell
commands. Committed cargo-dist0.30.0 archive digests and rustup1.28.2 executable
digests replace downloaded installer shells. npm11.21.1 has a committed SHA-512
archive integrity for deliberate lock generation; no force/legacy-peer flags.
PyYAML6.0.3 wheels have required hashes. The Glama reference builds the supplied
immutable source context, rather than cloning main. The precise remaining
mutable OS-repository/host-runner inputs and absence of a bit-for-bit image claim
are documented in `docs/BUILD_SECURITY.md`.

The proc-macro-error2 vendor source came from the exact original Cargo.lock
archive SHA256 `11ec05c52be0a07b08061f7dd003e7d7092e0472bc731b4af7bb1ef876109802`.
Only `extern crate proc_macro` became `pub extern crate proc_macro`; original
licenses/source metadata and patch receipt are retained. The root patch covers
all consumers, while the vendor crate is excluded as an independent workspace
product. `scripts/test-proc-macro-visibility.py` compiles an original negative
control and the corrected consumer with `-Dpub_use_of_private_extern_crate`:
original E0365 rejected, corrected accepted with0 future-incompatible warnings.
Evidence: `w7-vendor-regression.log` and `w7-proc-macro-regression/*.log`.
A direct workspace dependency-only `cargo check -p proc-macro-error2` exposed a
Cargo1.98.1 host-feature resolver panic; the standalone consumer and actual
backend consumer checks exercise the correction without relying on that broken
selection mode. This is not counted as a passing command.

Host checks: checksummed native cargo-dist and npm bootstraps executed and
reported exact versions (`w7-{dist,npm}-bootstrap.log`); real `dist plan` produced
all4 supported targets and the pinned focal index (`w7-dist-plan.json`).
`check-build-inputs.py` passed;7 build-boundary regressions passed, including
well-formed wrong download digest, unsafe archive traversal, mutable/unknown
Action pins, mutable base, unlocked build and generated installer rejection.
All workflow/Dependabot YAML parsed and reusable permissions check passed using
the hash-installed PyYAML. `cargo check --locked -p nyxid --bin nyxid-server
--future-incompat-report`, with bounded jobs2/debug0/incremental0 and the shared
target, passed in2m16s with0 future-incompatible dependencies
(`w7-backend-vendor-check.log`). Rust fmt and diff whitespace passed. The Oracle
worker now has a committed integrity lock and CI uses npm ci; lock generation
with the checked npm archive reported0 advisories (`w7-oracle-lock.log`).

This batch does not close W7: dependency remediation, source/image/secret scans,
actual shipped composition SBOM, mandatory signing/publication gates and distinct
nonpublishing OIDC rehearsal remain subsequent batches. PM's exact W5 image
acceptance is running. Its independent build comparison found preload ordering
divergence and an extra stock50x.html; those are open W7/W5 corrections, never a
reproducibility pass.

## W5 fixture event lifecycle correction

PM's actual nginx1.27.5 matrices passed22 responses each. Its unchanged-image
browser run passed21 tests and failed the unlisted-runtime test's required CSP
event collection; the functional denial passed, and Chromium trace recorded the
real connect-src rejection. Focused Linux reruns reproduced3/3 collector losses.
The auto-fixture depended only on context, allowing the test page to tear down
before queued DOM violation/exposeBinding delivery.

The fixture now depends on page to retain it through fixture teardown, drains
one browser task turn, then polls up to5 seconds for each exact annotated event.
Unexpected-event checking still covers every test; no assertion or exception
was removed. Host focused unlisted-runtime and injection cases, repeated10 each,
passed20/0 in23.6s against the unchanged W5 product dist
(`w5-csp-sync.log`, `w5-csp-sync-results.json`). ESLint passed. This is a test-only
correction; PM will apply the committed fixture onto its immutable product
snapshot and rerun, without rebuilding product.

## W7-R1-01 strict workflow parsing correction

PM found a duplicate `with` mapping in the coverage-backend-base Rust Action.
The permissive YAML parse reported above did not establish mapping uniqueness;
it could discard the toolchain input. The step now has one mapping containing
both toolchain1.98.1 and llvm-tools-preview, with its condition preserved.

All workflow YAML (both .yml and .yaml) is now parsed with a nested duplicate-key
rejecting SafeLoader, also reused by the reusable-permissions checker. CI runs
this guard and four regressions: duplicate conditional with, nested duplicate
permission, missing exact Rust toolchain, and the valid conditional step.
The guard passed7 workflows plus Dependabot and checked all14 Rust Actions for
explicit1.98.1. Four tests, reusable permissions (1call), and the existing
build-input guard passed. Exact commands/output: `w7-yaml-r1-01.log` in the
validation root. This correction supersedes the earlier permissive YAML
assurance; other W7 dependency/build changes remain separate.

## W7-R1-03 consumed fixture and generated container coverage

The build guard now reads Cargo's parsed dist metadata and validates actual
cargo-dist JSON plans before both release workflows consume their matrices.
It also covers .yaml Actions and centrally inventoried remote test images.
Eleven regressions passed (the original7 plus mutable Cargo container,
generated-plan container, remote fixture and .yaml Action cases).

Registry HTTPS responses were fetched for historical machine0.40/0.41,
updater0.41 and BusyBox1.37; each raw response SHA256 matched its registry
Docker-Content-Digest. The existing pinned Mongo8 and context-spike digest
responses were independently fetched and verified too. Receipts retain exact
URLs/platform indices/observed time in build-input-pins.json (16images total).
The migration harness pulls immutable old bytes then creates only the local
tag expected by its cfg(test) adapter. Production fixtures/mailbox execute
pinned bytes. Local newly built images remain explicit inputs. This does not
claim static resolution of arbitrary runtime image-construction expressions.

Exact commands/output: `w7-input-r1-03.log`;11 tests, real generated plan,
strict YAML/all14 Rust inputs, shell and Node syntax checks pass. The unchanged
W6 real Docker execution remains its prior evidence; final-source Docker
harness execution after these fixture changes is an integration check to run
with the final W9 machine source, rather than being claimed from syntax checks.

## W7 frontend dependency, repeatability and runtime refresh

The final frontend lock resolves Vite8.3.4 (retaining the baseline Vite8/Rolldown
path), Vitest/coverage4.1.12 and PostHog1.438.12, with patched transitive inputs.
The baseline31 npm advisories are now0 in the raw final npm audit. An explored
Vite7 downgrade was rejected after checking the actual baseline; its logs and
362-file output are exploratory and do not establish final acceptance.

JavaScript preload hints and docs traversal have deterministic ordering; the
actual Vite8 implementation separates CSS before the callback and appends its
original CSS order. The committed repeatability harness performed two complete
SPA/legal/credential builds at identical commit/epoch, thread pools1/8: all398
files match. Actual production-browser transport tests22/22 pass under host
nginx1.31.6 with the final dependency set. The complete frontend suite passes
581files/6170tests. Lint, renderer4cases, build-input guard and regenerated
wizard checks are recorded separately. The unused frontend pnpm lock is removed.

Both frontend and backend credential builders install and assert npm11.21.1
from the integrity-locked Node tools root; the pinned Node image's bundled npm
verifies that bootstrap. Host npm10.9.8 successfully installed that same root,
which reports npm11.21.1 and pnpm10.34.7. packageManager alone is not enforcement.

The runtime pins official nginx1.30.6/Alpine3.24.2 by raw-index SHA256
782bb89768cc87b7f95c18b95baa1af250e5d451f26794f59363540161147c01,
refreshes supported Alpine packages, and removes stock HTML before asset copy.
The unchanged base scans8 advisory occurrences; the real refreshed runtime
probe with python3 scans0 against the current prepared Trivy database. This is
runtime-probe evidence, not a scan of the complete final frontend image.
Repositories remain mutable inputs, so no whole-image reproducibility claim.
The exact final image must pass extraction parity, browser/header/entrypoint
matrices and scanning. The harness now requires nginx1.30, records comparison
failures and offers explicit image-dist acceptance without calling it parity.

Evidence under the validation root: w7-frontend-v8-final-{ci,audit,tests,lint,
wizard}.log/json as applicable; w7-frontend-v8-reproducibility/comparison.json
and both inventories/logs; w7-frontend-v8-browser.{log,json};
w7-frontend-policy-renderer.log; w7-frontend-input-guard.log;
w7-frontend-wizard-freshness.log; w7-node-tools-{resolve,ci}.log;
w7-nginx-refresh-build.log. Guest probe JSON is
/tmp/nyxid-w7-nginx-refreshed-trivy.json, with base/raw reports retained beside it.

Mobile R1-05/R1-06 remain OPEN. Candidate uncommitted patches and prior evidence
are preserved separately. Provider-rejected/interrupted verification is not a
pass; no implicated probe is retried or routed through another agent/model.

## W7 independent Node dependency roots

The SDK retains its baseline Vite7 demo line (resolved7.3.7) and updates
Vitest to4.1.12. Checked npm11.21.1 installed the lock successfully; the raw
SDK audit reports0 vulnerabilities (baseline11), all17 tests across3 files
pass, and all3 workspace builds pass. Evidence is `w7-sdk-{ci,tests,build}.log`
and the corresponding audit JSON in the validation root. These results are
independent of the still-open mobile findings.

The actual updater container transport now installs ws8.22.0 through a
committed npm lock rather than resolving an unversioned package during image
construction. Host locked install passes with0 reported advisories
(`w7-updater-npm-ci.log`); final container execution remains an integration
gate. The offline Claude plugin validator similarly installs from its complete
integrity lock using the reviewed npm bootstrap instead of a global npm
resolution. Locked install and raw audit pass0 (`w7-claude-validator-ci.log`,
`w7-claude-validator-audit.json`); no AI session/reviewer was launched.
Path filters and Dependabot include the independent validator, test transport,
Node tools and separate Cargo spike roots. Strict YAML (7 workflows,14 explicit
Rust toolchains), reusable permissions and build-input guards pass.

## W7-R1-07 offline validator execution correction

The prior locked install/audit success did not prove the default executable
worked: upstream's script-disabled bin is a placeholder. A repository launcher
now selects the installed integrity-locked native optional package, validates
its platform/version and permits only `plugin validate --strict PATH`. It never
executes upstream install/fallback downloads or opens a model session. Both
actual commands now pass locally: `scripts/validate-claude-plugin.py
--require-claude` with that launcher on PATH, and marketplace validation at `.`.
Evidence: `w7-claude-offline-{plugin,marketplace}.log`. Strict YAML and reusable
permissions pass. Frontend/SDK/coverage CI npm installs now explicitly use the
reviewed npm11 bootstrap, and updater test compilation adds the missing
`--locked`. This corrects the executable gap; the earlier install result alone
must not be treated as actual schema-validation acceptance.

## W7 shipped frontend composition and R1-08

The SPA, standalone backend credential page and CLI singlefile wizard now have
separate retained-module/CSS graphs and final-byte seals. Exact installed lock
name/version/integrity/purl identities describe retained package modules rather
than the whole dependency lock. Source identifiers are normalized; unknown
virtual/package mappings fail. Retained xterm/Tailwind/tw-animate CSS is tracked
separately from JS. Exact-byte worklets/theme assets and font/license provenance
cover unbundled outputs. Font versions come from the embedded name tables and
are not claimed as independently authenticated upstream releases.

Final HTML inline scripts/styles/style attributes and local/exact remote
references are accounted after Vite, prerender and singlefile transforms.
Vite-minified template CSS is bound to its original template block; captured
singlefile code must appear in final HTML. The parser rejects base elements and
active unaccounted attributes and recognizes case-insensitive rel tokens.
Sealing checks canonical lock names/purls as well as version/integrity. The
wizard source closure includes the parser, sealer and provenance policies.
`docs/SHIPPED_ARTIFACT_COMPOSITION.md` documents the acyclic ordering and keeps
future detached SBOM/signature bundles outside the sealed application tree.
These seals are inclusion/hash evidence, not yet artifact SBOMs or signatures.

Final focused composition tests pass11/11, including actual singlefile JS/CSS,
tree-shaken dependency absence, unknown/mislabeled package mapping, stale/missing
outputs, exact fonts/licenses, unmapped benign inline code, base and mixed-case
rel cases. Both final SPA/credential builds match402files under thread pools1/8
with Node22.23.2/npm11.21.1, full commit and commit epoch inputs. All three seal
verification modes pass. The Rust wizard freshness test passes1/0/0ignored.
Frontend lint exits0 with34 existing warnings. No invalid-selector or native
config warning remains in the final build; the large-chunk advisory remains.

R1-08 replaces the invalid escaped focus-class selector with the destructive
class plus focus state. All current destructive callers provide that class.
The committed production browser test focuses a real trigger menu item and
checks both its text and SVG against the actual built destructive color. Its
focused native-nginx run passes1/1 with the CSP collector enabled.
The complete corrected native-nginx production run passes23/23. Its first
run passed22 cases but failed the log/referrer case before assertions because
the command omitted `NYXID_NGINX_ACCESS_LOG`; that failed log is retained.

Evidence under the validation root: `w7-composition-{tests-final,build-final,
wizard-final,wizard-freshness,lint-final}.log`,
`w7-composition-repeatability/{comparison.json,build-*-sha256.json,build-*.log}`,
`w7-dropdown-browser.{log,json}` and `w7-composition-browser-final.{log,json}`.
Early seal failures exposed missing
template-CSS attribution and capture before aggregate CSS emission; both are
fixed and covered by the final tests. No blocked mobile probe was retried.
Final exact-source image extraction/scan/browser checks remain PM-owned.

## W7-R1-10 reviewed Node install scripts

The five updated Node roots now disable dependency lifecycle hooks through
committed `.npmrc` files and record explicit denied packages in `allowScripts`.
The review catalog binds each script-bearing dependency to its exact lock path,
name, version and archive integrity, with a reason and supported execution
path. The guard rejects unknown or changed hooks, stale reviews, omitted roots,
and blanket approval. Docker copies the policy before installation. SDK
workspace commands use the supported `--workspaces` spelling. Wizard freshness
also binds the frontend install policy.

Fresh reviewed-npm11 installs pass with no unreviewed-script warning:
frontend614 packages/audit0, SDK98 packages/audit0, tools146 audited/audit0,
and offline-validator/updater transport installs pass. Actual SDK tests pass
17/17 and all3 workspace builds pass. Composition tests pass11/11, wizard
build/seal passes, and Rust wizard freshness passes1/1 with0 ignored. Actual
offline plugin and marketplace schema validation both pass. The policy guard
and its4 regression tests pass; workflow/build-input guards pass. Evidence is
`w7-{frontend,sdk}-script-policy-ci.log`, `w7-sdk-script-policy-{tests,build}.log`,
`w7-frontend-script-policy-tests.log`, `w7-script-policy-wizard{,-freshness}.log`,
`w7-claude-script-policy-{ci,plugin,marketplace}.log`,
`w7-tools-script-policy-ci.log`, and `w7-updater-script-policy-ci.log` in the
validation root. SPA and wizard executable bytes are unchanged by this policy
batch. Mobile's interrupted patches remain outside this acceptance claim.

PM classified all34 frontend lint warnings:30 fast-refresh export structure,
3 existing hook dependencies and1 TanStack Virtual compiler compatibility.
All21 warning files are blob-identical to baseline; there is no new composition
or selector warning. The existing chunk advisory is also preserved separately
from this batch's acceptance evidence.

## W7-R1-12 module identity boundary correction

PM's exact73886f25 container build failed because the sealer searched every
normalized module identifier for the checkout-root string `/build`. That
incorrectly rejected first-party `build-version.ts` and legitimate package
`build/modern` directories. The sealer now parses repository/package/generated
identity boundaries and validates relative path components before resolution.
Absolute, drive, empty, traversal, control, backslash and raw node_modules
identities fail; generated helpers retain their explicit tool mapping. Errors
do not print local source paths.

Focused composition tests pass12/12, including synthetic root `/build`, both
valid module forms and invalid absolute/traversal forms. All3 existing final
seal verify commands pass without changing their bytes. Regenerated wizard
freshness passes1/1 with0 ignored. Evidence is
`w7-composition-r1-12-{tests,freshness}.log` in the validation root. This is a
host correction; the failed738 image log remains preserved, and PM owns the
exact descendant container rerun. No functional-image pass is claimed here.

## W7 final Rust metadata, standard SBOM and bootstrap batch

The shipped Rust Docker builds and cargo-dist metadata now require the pinned
cargo-auditable0.7.7 tool. The shared bootstrap selects Darwin/GNU/musl by the
actual host ABI and rejects unknown/unavailable per-tool targets (R1-14).
Generated installer admission rejects install_cargo_auditable in dot and
bracket syntax (R1-13), alongside existing generated dist/package commands.
The isolated locked artifact-tools root uses maintained auditable-info0.10.1,
has Dependabot/fmt/Clippy coverage, and its actual Cargo dependency scan passed.

Actual host dist builds with strip=true passed: updater4m43s and CLI8m15s.
The final updater Mach-O is11276944bytes, SHA256
dbeb3b99e24ed29dcd76cbaebe047827f2681cccd9ba4e4100cb4582e8414f73,
with353 metadata packages (301runtime/52build). The final CLI Mach-O is
33215904bytes, SHA256
7b2e3f7ef6053fef2c2313ce21bbfe2c468af5e4eea1d1ac11c8b5265cd6d007,
with541 packages (485runtime/56build). Independent pinned Syft1.54.1 inventories
match both runtime sets. Stable Cargo metadata can conservatively include
extra dependencies generally; this is not per-crate emitted-byte attribution.

The CLI final binary contains all9 sealed wizard assets, including its seal;
the raw seal SHA256 is
f54eca14dbd0fd6d0eace932fb4c231ee17f479cba10082df2d8882bbef52f3c.
The CLI CycloneDX1.6 document has614 components including the retained JS/CSS,
static files and HTML provenance; updater has353. Both actual documents pass
the pinned official schema offline. Registry purls are limited to authenticated
registry-source metadata; local patched code has exact reviewed-file/patch and
upstream-ancestor pedigree. Unsupported git/custom origins fail. Pedigree does
not exempt the dependency/version from source/ancestor vulnerability scanning.

Pinned Alpine rust1.98.1 image bootstrap selected the actual musl tool, compiled
a locked two-package fixture, stripped it, and ran it with expected output42.
The maintained parser reads its retained ELF metadata. Native guest GNU
bootstrap and the same locked fixture compile also pass. These fixture runs
establish ABI selection and stripped-ELF handling; they do not replace final
application ELF/image acceptance. The scratch context and exact commands are
retained in w7-auditable-alpine-context.tar and the adjacent build/runtime logs.

Focused checks pass: build-input14, embedded-byte3, binary-SBOM7, official-schema3;
artifact-tools fmt/Clippy, strict workflow YAML and existing build-input checks
pass. An actual executable without dependency metadata is rejected. The new
validator has6 integrity-locked Python packages (159official wheel hashes),
requires Python3.9+, and runs under explicit Python3.12 CI. It is not installed
in focal's Python3.8 local-build job. Official CycloneDX schemas/LICENSE resolve
to55343ba19dee1785acf1ce9191540d5fd7b590db; PM independently matched all schema
bytes and159wheel hashes.

Evidence in the validation root: w7-auditable-{cli,updater}-build.log,
w7-{cli,updater}-binary-{info.json,syft.json,cdx.json} (the actual names use
binary.syft.json/binary.cdx.json), w7-cli-{wizard-seal.log,embedded-wizard.json},
w7-auditable-alpine-{build.log,runtime.log,binary-info.json},
w7-auditable-gnu-bootstrap.log, w7-artifact-tools-trivy.json,
w7-artifact-missing-metadata.json and w7-rust-*-tests.log. These application
binaries are working-batch subjects built on1dbac798 with the recorded source
diff; they are not signed clean-release provenance. Actual final archives,
all four CLI platforms, final backend/machine subjects, immutable distro
packages and mandatory producer/signing/scanning gates remain separate work.
Mobile R1-05/R1-06 remain OPEN and untouched.

## W7 expected subjects, archive and OCI metadata preparation

The complete publication contract now starts from an explicit policy and
cross-checks actual cargo-dist/Cargo or image-workflow matrix inputs. It expects
all4 CLI targets (13file subjects including releases.json) and5 image components
on both Linux architectures (10runnable children+5indices). Structural
validation rejects reduced scope, missing/duplicate/extra subjects, stale
commit/epoch/target identities, wrong index children, and required jobs whose
result is not success. Evidence closure checks hash the exact required files;
they explicitly do not validate signatures or scanner semantics.

The archive inspector rejects unexpected/nonregular/traversing members, bounds
archive expansion, inspects executable format/architecture, and requires the
planned GNU loader for GNU Linux targets. It independently extracts dependency
metadata/Syft/wizard bytes and emits distinct archive and binary CycloneDX
subjects. Local target subsets or source diffs are marked local-investigation.
Retained built-source identity (R1-15), complete final archive coverage, actual
scan/signature validation and final producer integration remain OPEN. A plan's
commit label is not proof that a binary was built from that commit.

OCI metadata validation was exercised on raw digest-preserved JSON captured
from the actual default-isolation Alpine Buildx fixture. The build envelope
index is2d2742df32b15958e9a12698192ad4c1ac18029d937a7eb36c45d51b6d06a0c3,
runnable child ace9035facb8cad4660661cd35ca7fb2bcfd6613198d0278afcc6adaf8d30ec0,
auxiliary28652dc776e3b9b5953e0cf2f75bf6b04a94430b78b78fb82b3ce394a3e4c74d.
The validator preserves each identity, validates exact runnable platforms and
metadata digest/size/config-revision bindings, and permits only explicitly
supported bound BuildKit auxiliary statements. The unsigned embedded statement
does not substitute for GitHub OIDC provenance. Filesystem layers are omitted
from the metadata fixture and are not scanned by this test.

Focused regressions pass8expected-set+5archive+6OCI tests. They include missing,
duplicate, extra and stale subjects; skipped/failed signing jobs; removed or
changed signature evidence; wrong architecture/libc/executable kind; unknown
and unbound OCI auxiliaries; and supported arm64/v8 vs unsupported variants.
Actual regenerated plans contain13/15subjects, and the current source-aware
wizard seal verification passes. The cargo-dist archive compile in
w7-actual-dist-archive-build.log is ongoing tooling evidence only: main Rust
dependency findings and R1-15 must be resolved before final archive acceptance.
Commands and limitations are documented in docs/PUBLICATION_SUBJECTS.md.

PM's exact18cd source scan reports53package/advisory occurrences,48unique
advisories across21package/version pairs; optional gcp-kms includes the older
jsonwebtoken path. Supported Rust remediation and meaningful W6/SSH/crypto/KMS
caller checks are the next implementation priority. The116fully redacted
Gitleaks candidates require specific classification and negative controls;
they are not116confirmed leaks. Mobile R1-05/R1-06 remain OPEN and untouched.

## W7 R1-17 OCI platform ancestry correction

PM's metadata-only review found that a nested envelope could override an outer
platform declaration, and a direct manifest without a descriptor platform
could bypass config-variant policy. Validation now carries every ancestor
platform declaration to each runnable config and requires all to agree under
the supported arm64/v8 normalization. Config variant policy applies even when
no descriptor declares a platform. Focused OCI tests pass8/8, adding valid and
mismatched nested-envelope cases and direct-manifest absent/v8/v9 cases. This
correction does not establish final producer/signature/scan acceptance.

## W7 supported Rust dependency remediation (working batch)

The main Rust graph now uses maintained MongoDB3.9.1 with BSON2 compatibility,
Hickory0.26.3, Tough0.22, Sigstore0.14, Russh0.63.3 and lopdf0.42. The GCP
provider uses the same upstream's maintained gcloud-kms1.5 package through its
existing Cargo alias; AWS uses the SDK's modern default HTTPS client. Supported
owner updates remove the old libcrux/OpenSSL paths and update the affected TLS,
QUIC, random, archive and parsing dependencies without advisory suppressions.
CI Clippy/backend tests include both optional KMS features and use --locked.

Caller changes preserve bounded Tough streaming, existing raw SSH host-key pins
and persisted TOFU, and explicitly reject negotiated host certificates. The
actual local SSH wire suite passes12/0, including a successful backend-style
OpenSSH user-certificate authentication after raw-key authentication refusal.
Host-certificate rejection and user-certificate authentication are distinct
controls. GCP tests exercise local gRPC current/previous wrap/unwrap, denial and
retry handling, plus an RSA-signed service-account JWT/local OAuth exchange.
AWS fixtures use explicit dummy credentials and local HTTPS with a fixture CA;
production TLS/default client construction remains unchanged. The isolated AWS
module harness passes2/0 but does not substitute for the integrated backend run.

The first both-KMS integrated backend executable passed246crypto,455model,
5PDF/DOCX parent,1live Mongo transaction,46key-service and12agent-binding tests.
The one attachment ignore is a subprocess entrypoint invoked by its parent,
not an unavailable database test. Actual Mongo commit/abort, key rotation replay
and concurrent binding mutation checks executed against Mongo8.0.17. This
first executable predates the added AWS transport module/direct dev dependency.
The final both-KMS rebuild passes and its executable passes248crypto (including
the2actual AWS HTTPS tests),455models,5PDF/DOCX parents,1live transaction,
46key-service and12binding tests, with the same one child-entrypoint ignore.
The first full workspace Clippy failed on clippy::err_expect in the new GCP
denial fixture. Its failed log is retained. The only source correction replaces
Result.err().expect(...) with Result.expect_err(...); the exact integrated GCP
test then executes1/0/0, and corrected full workspace Clippy passes with both
KMS features, all targets, --locked and -D warnings. Formatting, whitespace,
strict workflow YAML, reusable permissions and build-input checks pass.
The macOS linker reported a large __eh_frame performance warning; the build
and these tests exited0. This is no Linux backend/image execution claim.

Final working source is basee30188e2 plus19effective reviewed overlays:
w7-rust-updated-tests-source-v4.tar SHA256
dc54d8c1952f8d9f2761167dc7ae25c1b4cc0bf575895e39e893462a4a8027c6.
The sole test-only Clippy correction is frozen as v5 tar SHA256
6bea5bcd4aea7d239897ab04614a7b9c6d4660b4e75eaa2ff28f368d7d340351;
all19effective overlay hashes match the bounded dependency code being committed.
The unchanged broad groups retain their v4 executable provenance; the corrected
test and Clippy receipts bind v5 separately. No result is relabeled as a final
release/archive/image run.
Cargo.lock SHA256 is
0fd7d32fb7386f4dbbb08098cfc2edcb53a24d82ce951de8773619cd61c6d783.
The final guest updater suites pass120/0/1library and79/0/4binary. Each live
TUF test was also explicitly executed: CLI1/0, updater library1/0 and updater
binary1/0. The CLI's other verifier cases pass40/0/1. Binary updater ignores
are that same live TUF case, one companion child entrypoint and two Docker
parents; the child/parents remain pending final container integration.
Initial guest permission-fixture failures are retained. Test-only helpers now
create private files/directories explicitly while the invalid0666/0770,
missing-token and no-repair controls remain; production policy is unchanged.

Exact final-lock Trivy0.76.0 scan passes792packages/0vulnerabilities with raw
report SHA256
98efb01748fb00871051d262515c6a07c7be09b4181c66f9538f20bb10fab447.
This is a source dependency scan, not a final rebuilt image/archive scan.
The exact final command arguments, explicit environment, exit status, executable
SHA256, each group log SHA256 and frozen overlay hashes are retained in
w7-rust-remediation-final-receipt.json under the active nyxid-w5w9-validation
directory. Final build: cargo test --locked -p nyxid --bin nyxid-server
--features aws-kms,gcp-kms --no-run, with CARGO_BUILD_JOBS=2,
CARGO_PROFILE_TEST_DEBUG=0, CARGO_PROFILE_DEV_DEBUG=0, CARGO_INCREMENTAL=0 and
the isolated validation CARGO_TARGET_DIR. Direct group tests use --nocapture
and NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:27189/?replicaSet=rs-security.
R1-15 application wiring/actual stripped
artifact identity, immutable distro inputs, producer signatures/scans and full
final artifact coverage remain OPEN. Mobile R1-05/R1-06 remain untouched OPEN.

## W7 R1-15 source identity and exact source export tools

The shared helper records full commit, commit epoch, actual Cargo package
version and target in a retained ELF/Mach-O section. Git working-tree builds
are conservatively git-unverified or known git-dirty; a cached build script
cannot certify clean source when another package introduces a new build input.
The release contract accepts exported source only. Unknown, dirty, unverified
and the former git-clean state cannot satisfy official publication. Actual ABI
inspection remains separate from the declared target string.

Focused helper tests execute stripped fixtures and the actual newly rebuilt
artifact-info extractor. All seven pass, including unchanged Fresh controls,
unstaged sibling dependency edits, linked worktrees/packed refs and optional
untracked input creation/removal (actual outputs 2 -> 3 -> 2). The earlier
new-input control failed because its diagnostic git status refreshed the Git
index; that failure is retained. GIT_OPTIONAL_LOCKS=0 makes fixture inspection
read-only; no assertion was removed. Helper SHA256 is
39cb596250af4e0cdf90c2a5e3e5d6d06864b7f96a81462c3a59076ce6ddc592;
test SHA256 eeb58551266f23f4601ca3d33afdb511d96adf48530a06d3044996114d630675.

The new exporter validates a clean exact checkout, reads raw committed blobs
and writes a deterministic archive outside Git. It deliberately applies no
export-ignore/export-subst, local/global attribute or working-tree filter
transformations. Receipt verification reconstructs the committed Git tree,
checks the raw commit object against the expected commit/tree/epoch, validates
each archive blob/mode and compares the extracted source to those committed
inputs. Reviewed generated-output exclusions cannot hide tracked inputs.
Local info/attributes can make ordinary git archive omit a clean committed
file; the regression reproduces that omission and proves the protected export
is unchanged. Ten exporter tests pass, including new files/empty directories,
archive/source/receipt tampering and symlink/output boundaries.

Exact command/env/exit/log receipts are in
w7-source-identity-stable-tests-receipt.json and
w7-identity-parser-stable-receipt.json under the active validation directory.
Publication contract tests pass9/9, archive tests5/5, actual parser tests2/2.
This bounded tools commit does not claim final application/builder/archive/
image identity. The working application wiring passed locked cargo check for
backend/CLI/updater bins with both KMS features; it remains a separate working
integration snapshot. Official producer export/build wiring, actual stripped
application subjects and full final evidence remain required for R1-15.

## W7 R1-15 application/source packaging and required build wiring

Backend, CLI and updater build scripts now use the shared retained identity
helper, and all three main entrypoints retain the marker. Docker builders copy
the helper and require full commit plus commit epoch; backend credential build
uses the same inputs. Official CI/release/cache/image builds export and verify
raw committed source. Supported local image commands now use the same export
wrapper, and node Compose consumes the explicitly prebuilt local image.
Dirty local Cargo builds remain honestly unverified/dirty. This bounded commit
contains identity-only Docker changes; snapshot APT/APK work remains separate.

Pinned cargo-dist0.30 cannot package its default Git source task from a no-Git
export. Supported source-tarball=false, precise-builds=true and an explicit
extra-artifact hook now package deterministic gzip of the exact verified raw
source.tar. The hook never uses Git archive, attributes or generated source.
Actual full-tree no-Git global packaging passes exit0 in10.633s on committed
integration fixture22590776, with source/export/hook/archive/manifest receipts.
This proves packaging compatibility, not a complete release.

Pinned dist also ignores required Cargo child completion. Its supported CARGO
override now enforces --locked, exact target/package/profile, actual child exit0,
one successful BuildFinished stream and hashes of produced executables.
The producer requires fresh staged archives whose binary bytes match those
completed children. It verifies all four local target receipts before global
packaging and checks the fresh source-hook result. The pinned rust-cache cleanup
retains empty directory trees: the producer removes only directory-only residue
within the two reserved staging roots, records it, and rejects/preserves any
stale file, symlink or other object before deleting anything. Dependency cache
files remain available. Failed/cancelled child plus outer0 and an old same-source
binary cannot satisfy completion. Required local/global workflow jobs must
actually succeed; skipped/failed jobs cannot authorize host publication.

Actual tiny CLI fixture d401230d passes reviewed dist -> guard -> auditable
Cargo -> stripped Mach-O archive in5.364s, with completed child and archived
binary SHA equality. It is deliberately a dependency-free fixture, not a product
binary. Original v1 checksum-validation failure and v2 receipt remain intact;
fixture-input supplements distinguish the original product template manifest
from exact executed tiny root/CLI manifests, main/build source and lock hashes.
The final working application check passes cargo check --locked -p nyxid-cli
-p nyxid-machine-updater -p nyxid --bins --features nyxid/gcp-kms,nyxid/aws-kms
in111.963s, with nine unchanged before/after input hashes. It is compile evidence,
not final stripped application/archive/image acceptance.

Frozen validation tree d0063ec755e2126ce24e44f5a5c73cede62b97e0 passes exporter10,
identity/cache7, source-hook4, exported-execution7, required-completion6,
publication10, archive5, local-image3, actual tiny-Cargo embed-guard4 and
build-input15 tests. Strict YAML/reusable/build-input, shell/JS syntax and
whitespace checks pass. Exact command/environment/exit/log hashes are under
w7-source-wiring-{source,producer,guards}-receipt.json; the aggregate
w7-source-wiring-handoff-receipt.json binds these, the actual working compile,
global packaging and separate tiny fixtures. Node Compose structural assertions
pass; actual Compose CLI is unavailable on both host and guest, so no Compose
runtime/config command pass is claimed. Release credential workflow now verifies
the wizard source/final-byte seal; final archive inspection's independent embedded
byte check still needs the complete final security-gate wiring.

Final exported stripped app subjects, all-platform archives/images, package
input receipts, SBOM/scanner/signature semantics and complete staged publication
remain OPEN. No remote workflow/publication or implicated mobile probe ran.

## W9 common new-key issuance defaults and client contract

Ordinary key creation now resolves omitted/null/empty lifetime to 90 days and
omitted service/node allow-all flags to false. Explicit nonexpiry carries typed
issuance intent; managed NyxBot/conversation and directly constructed shared-entry
transport authority retain deliberate nonexpiring policy. Stored model defaults
are unchanged, including legacy missing/null lifetime and broad grant defaults.
Rotation retains lifetime and grants and returns the stored expiry. Create/rotate
metadata uses BSON millisecond precision; explicit fractional intent stays exact
through scheduled scope-plan digest verification before storage normalization.

Agent Key/auth-device approvals resolve parent and child intent before retryable
transactions; new children default finite, bounded by the parent. Login-code mint
persists the frozen intent; old-shaped pending rows derive defaults from stable
row creation time. Stored child null remains null during delivery/authentication.
False never_expires is omitted on wire and persisted selection. The genuinely
new true intent requires all backend readers/issuance clients to upgrade; the
cutover document requires pausing/draining issuance, approvals and code mint/
redeem during rollout. It does not claim mixed-version runtime enforcement.

Dashboard setup/reset/open and personal/org transitions retain restricted finite
defaults. Shared hosted/local wizard now resets its effective visible lifetime on
owner change and permits a fresh explicit nonexpiry choice. Original frozen v1
owner regressions failed 2/2; corrected frozen v2 passes those unchanged controls
in PM scratch. CLI omission/positive/zero and wizard allowlist/prefill preserve
intent. Ordinary mobile UI/schema changes retain safe defaults and explicit
nonexpiry without executing dependency/patch probes. Creation success reads the
returned expiry and distinguishes an absent old-server field from null. Shared
wizard/terminal rotation summaries omit lifetime; docs state that limitation.
Device provisioning separately exposes api_key_expires_at while expires_in keeps
its existing delivery-envelope meaning; polling looks up expiry before claiming
one-time delivery.

Client validation: corrected frontend focused suite86/7files, additional callers
45/4files, TypeScript and scoped ESLint pass. Ordinary mobile12/0skip and TypeScript
pass on unchanged client bytes with existing modules; no mobile dependency edits
are included. Wizard rebuild/final-byte seal pass. Existing unchanged compiled
wizard freshness executable also passes, explicitly a reused test executable.
Frontend tests retain PostHog sourcemap warnings; scoped ESLint is zero-warning.
PM independently verified frozen v2 overlay36files, focused86, same owner
regressions2, separate callers37, TypeScript/lint/seal; these are separate runs.

CLI debug executable7a6358f7a29f70aca25c112fd6cde593bdf294e8904e49c403503de9a56bf104
passes key35/device13/pairing55/local-wizard68/parser34 =205/0/0; PM copied and ran
all five filters independently with equal results. Debug RustEmbed reads assets
from source, so this is not final release embedded-byte proof. Original verbatim
v2-byte probes false are retained with the rust-embed cfg explanation.

Final both-KMS backend test compile passed on frozen source9949575f in914.977s;
its immutable debug executable0980b6c0 is retained. Corrected-source final CLI
no-run also passed and reproduced exact executable7a6358f7. Both-feature backend/
CLI bins/tests Clippy with -D warnings passed in1446.790s. The backend link retains
the __eh_frame/compact-unwind size warning; no runtime correctness failure is
inferred from it. Scoped rustfmt and git diff --check pass.

PM full frozen-v2 frontend execution records6187pass/3fail. All three failed
composition mapping builds resolve installed-package symlinks outside PM scratch;
unchanged source with copied dependencies passes all14 tests in both failed
files. A complete33656-entry dependency inventory matches apart from caches.
The original full-suite failures remain; no full-suite zero-failure claim.

Backend runtime and final fixture-only validation are complete within the scopes below.
The initial key-service run is43pass/1fail: an old fixture attempts new issuance
already expired, which the new policy correctly rejects. The reviewed final fixture
correction issues a valid key and then expires its stored row before testing
authentication. The original failure remains. During Agent login group05,
Mongo exhausted its8192FD process limit and exited. That run was interrupted,
not passed. Only local Mongo8.0.17 was restarted, using the same dbpath/replica set
with process limit65536, followed by real commit/abort readiness checks and no
existing-database deletion. The exact0980 matrix replay completed on that endpoint. Agent Key login28/0/0, auth-device7/0/0, login-code9/0/0, device provisioning45/0/0,
assistant actions8/0/0 and managed credentials10/0/0 completed after restart.
Permission11, scheduled fractional1, key mutation5, key scope7, durable grants7,
machine-source13, machine-runtime3, org membership8 and platform setup6 also pass;
NyxBot128 also passes. Across20 distinct groups the exact0980 snapshot records
391pass/1 retained expired-fixture failure/0ignored. The original groups01-04 and
replayed05-20 receipts remain separately bound. Restart and original crash receipts/logs
remain distinct. PM independently copied exact0980 and passed10/0/0 selected
actual HTTP/precision/rotation, BSON legacy, parent/child/auth-device, transaction
conflict, login-code frozen/rollback, assistant rotation, permission and scheduled
fractional-digest cases; that selected run is separate from the worker matrix.
All original failed frontend/types assertions, initial backend no-library-target
command, prior-source successful compile and deliberately interrupted later build
remain in separate receipts. No interrupted build is a passing runtime result.

Final v3 source4336c599699cc60fc8ddc239c6cf3f8595991b6064429406cbd43233651ad1fb
differs from frozen v2 in only two reviewed test sections: issue-then-expire
fixture setup and expanded real HTTP empty-expiry/false-gates table. Rustfmt
changes in those sections are retained in the exact delta receipt. All other32
backend/CLI snapshot inputs, including production prefixes and manifests/lock,
remain unchanged. The final binary build, affected groups and Clippy results
are recorded below; broad0980 execution retains its original executable scope.

Final v3 both-KMS backend test build passes in594.479s with actual successful
BuildFinished and unchanged source inputs. Copied immutable test executable
12b77ea53bdb96689c6c60e0bb77a2b7422b4d951fd8cca80bd8935183cb86c1
passes full affected key-service44/0/0 and API handlers41/0/0. The corrected
expired-auth test retains Unauthorized after explicitly expiring a valid stored
row. Expanded HTTP table covers empty date and explicit false gates alongside
exact returned/stored precision and unchanged rotation lifetime. Runtime source
and executable hashes stay unchanged; no final tests are ignored.
PM independently copies that exact executable and passes the two changed exact
cases2/0/0 on the healthy configured Mongo endpoint, with source34 unchanged.
Its final review binds the worker build and both affected-group command/log
receipts. This is separate from the earlier exact0980 broad matrix and the205
CLI tests; no results are reassigned between executables.
Final scoped Clippy passes exit0 in386.242s on the consistent final source:
cargo clippy --locked -p nyxid -p nyxid-cli --bins --tests
--features nyxid/gcp-kms,nyxid/aws-kms -- -D warnings.
Its exact argv/environment/input/exit receipt and log SHA256
4c01a4dba853a692ff808727af7cbaf33471a58a4ff420ca761123f8f6c76a45
are retained. Final scoped rustfmt and whitespace checks pass. The complete
w9-final-handoff-receipt.json indexes53 command receipts, frozen source/delta,
old and final executable scopes, failure accounting and PM evidence under the
active validation directory. w9-final-bounded-source-hashes.json binds the64
implementation paths separately from this worker log. No unrelated Docker,
APT/APK, mobile dependency/patch or PM-ledger work enters this bounded commit.

This batch implements the common creation/lifetime contract; it does not close
W9.3 readiness/required-approval, W9.4-6 OS storage/migration, W8 durable audit,
or W7 shipped-artifact/source/package/scanner/SBOM/signature/producer gates.
R1-05/R1-06 remain untouched OPEN. No remote workflow, signing/publication,
live credential use or implicated mobile probe occurred. Next implementation
is W9.3 readiness and usable required approval; queued W7 Node/npm scratch
preparation remains separate.

## Remaining authorized batches

W5 final container review/integration; W7 pinned,
locked and scanned builds, shipped-artifact SBOMs and mandatory official manifest
signing; W9 readiness, real OS identity/key-service
boundaries and resumable full secret migration; W8 strict attribution, complete
critical-operation durability, independent protected witness, prefix retention,
key lifecycle and scoped API/UI. Major W8 code follows a concrete operation and
architecture proposal sent to PM. No scope row is closed by this log.


## 2026-10-19 hosted CI regression corrections

Draft PR 1944 CI exposed three tooling/test-environment failures while the
separate W9.3 frozen backend runtime matrix remained active. This bounded batch
changes four CI/source-tool files and this worker log; all 1,293 frozen backend
and CLI compilation inputs remain byte-equal. Original hosted logs are retained
under the validation directory's `pr-1944-ci/`: identity job 117900846543
(dbee19558e0b61c7458c5f6688abc58c35eee9d68e7b0563db57e933558f573b), ARM cache
job 117900985981
(f9f110f5c355311ae1367044ec3ee71bcd38c4a71077094ffa93dc7eb79f8f91), and Detect
Changes job 117900846695
(ef2d0f51a37db5d2008b668fdcd772ad51e5ca3921bcc4d9360b6ccfc4c1cf52).
The cache failure actually ran against PR merge commit
448139ac6ce6e04c3575caadc91a882244fec552, distinct from head 473eccf1.

Cargo's `CARGO_TERM_COLOR=always` inserts an ANSI SGR reset between `Fresh` and
the package name. The original assertion was reproduced locally with the same
color setting and failed both sibling-helper variants. Freshness assertions now
strip only SGR formatting for comparison, retain raw stderr, and still require
the expected package to be Fresh with no build-script invocation. All seven
identity controls pass under that actual color environment. No artifact
extractor was supplied for these focused runs; earlier parser/application
boundaries remain separate.

The exact exporter intentionally ignores inherited Git configuration, which
also removed checkout's global safe.directory allowance for a container-owned
checkout. Git commands now trust only the caller-selected resolved directory
for that command. Global/system/alternate-index/replacement isolation remains.
Git's own different-owner test control reproduces an untrusted checkout, then
passes full exact export/verify with the narrow trust. An unrelated repository
is still rejected, and an ownership error checking a nested candidate fails
closed instead of becoming a successful no-Git assertion. Eleven exporter
controls pass, including all existing dirty/untracked, committed-tree/blob,
attribute, archive, symlink, and generated-path rejection cases. The dependent
exported-runner seven and source-hook four controls also pass.

The cancellation fixture's ready deadline was 15 seconds, while four optional
metadata probes can take five seconds each before child spawn. A forced local
control with four four-second probe shims reproduces the exact ready failure at
15.035 seconds and records the probe start times. Both cancellation fixtures
now use deterministic metadata/snapshots in their test subprocess only;
production resource collection is byte-unchanged. The original ready deadline,
return-143 requirement, bounded cancellation/grace, and descendant-stopped
assertions remain. Both fixtures and a separate metadata probe/whitelist test
pass under the same slow-probe PATH (three tests); all 21 Detect Changes Python
unit tests then pass. The local forced cause is supported; the hosted log does
not contain individual probe timings, so no exact hosted timing is invented.

Exact argv, cwd, selected environment, exit, elapsed time, source maps and log
hashes are in validation receipts `w7-ci-identity-color-final`,
`w7-ci-export-boundary-final`, `w7-ci-exported-runner`, `w7-ci-source-hook`,
`w7-ci-cancellation-isolated`, `w7-ci-cancellation-exact-env-final`, and
`w7-ci-detect-changes-final`. The unchanged three-case final control also
records the exact slow-probe PATH, PYTHONPATH, and trace-file environment; the
first three-case receipt recorded only selected Cargo fields. The two
preliminary forced-failure commands retain raw logs/tool exits but did not have
separate outer timing receipts; that limitation is explicit. Frozen handoffs
`w7-ci-color-and-export-v1` and `w7-ci-cancellation-v1` preserve their exact
source/evidence boundaries. PM independently verifies the three-file
color/export snapshot and passes seven identity plus eleven exporter controls
in scratch; its receipt is
`P/w7-ci-color-export-review/completed-review.json`
(3aad574aab2d57a7196e14386b51d5c4a646e5f3c752baf4c031c41bf6dc1292).

These are local CI-tooling corrections, not hosted ARM cache/producer execution,
application release identity, final artifacts/images/scans, W9.3 acceptance,
or a CodeQL zero-finding result. Telegram vendor bytes and CodeQL policies are
unchanged; precise consumer/alert disposition remains separate. No remote
workflow, publication, signing, merge, guest, live credentials, or blocked
R1-05/R1-06 probe was invoked by this batch. The original W9.3 failures/matrix
continue under their frozen earlier executable and source attribution.


## W7 bounded hosted CI followup: old Git trust and exact Rust Action setup

The next hosted PR merge checkout (adf10b1865ed032174b70c45e95acd2aab8b7764,
head574800f0 plus main3fc6386a) retained two distinct failures. Focal Git2.25.1
still rejected the container checkout despite command-local safe.directory;
its security backport accepts the setting from global files but does not
implement GIT_CONFIG_GLOBAL. The other job installed minimal Rust1.98.1 and
then hit a clippy-preview cargo-clippy conflict when the repository override
downloaded missing components. The pinned version-specific Rust Action warns
that toolchain is unsupported: its implementation hardcodes1.98.1 and accepts
only target, targets, components. Original hosted logs remain in validation
pr-1944-ci/117905729002-download.log and117905569623-download.log with hashes
1d449fc5471155f30a7c11ead9530d9ad3f87b7f82660238fb610fdef4f1115d and
dd2bfad52d6475b88a75cb62292623d39786ef80955632c37979d6dc1a88e187.

The exporter supplies a private XDG git/config and the same modern global
config file, containing only the exact resolved checkout safe.directory.
Git subprocesses omit inherited HOME, XDG, GIT overrides and SUDO_UID; no user
config or checkout ownership changes. Terminal literal-star roots and symlink
aliases resolving to them fail before either trust form can interpret /* as
subtree trust. Newline/tab and quoted/backslash/space/injection-text values
parse to exactly one intended config value. CR and other unsupported controls
fail early. Hostile HOME/XDG includes are not read; private config is removed
after body or Git errors. The modern different-owner control and actual
differently-owned Focal fixture still reject unrelated repositories.

All15 reviewed Action invocations remove unsupported toolchain and explicitly
install repository rustfmt/clippy during setup, retaining extra components and
targets. The narrow workflow guard requires the exact reviewed Action SHA,
supported inputs and repository components. Workflow triggers, permission and
publishing conditions are unchanged. Native clean-Rustup probes establish the
setup contract but do not reproduce the hosted Linux component conflict.

Final local checks: host Git2.50.1 exporter15 and offline cached Focal
Git2.25.1/Python3.8.10 exporter15; exported-runner7; source-hook4; strict YAML6;
build-inputs15; workflow and reusable-permission guards pass. Exact source,
commands, environments, exits, elapsed times and log hashes are bound in
validation w7-ci-oldgit-action-subset-v2.json (a22139af608633475898ea1e1a999628b4e363570445d5c7b386bca1acb98b11),
archivebc674fc8f4b1d0d48aa8a8961d7d9a4c7246aae1b354809775a440cb096cd99a.
PM independently reviewed all eight frozen code paths and ran exporter15/YAML6
in scratch at P/w7-ci-oldgit-review/completed-v2-review.json.
Earlier snapshots, failures and carried validator input attribution remain
distinct; build-inputs has15 tests (an earlier message's5 was shorthand error).

This commit is bounded to eight CI/tool paths and this worker log. It provides
local compatibility and regression evidence; hosted ARM export/component
execution, full producer/final artifact/image/scan gates remain open. The W9.3
compiled sources, CodeQL consumer WIP/vendor bytes, unrelated package/mobile
work and PM ledgers are excluded. No merge, push, release/signing, alert
disposition or blocked R1-05/R1-06 probe is performed by this commit.


## W7 bounded CodeQL consumer and telemetry-observer followup

The actual JavaScript SARIF at PR merge448 identifies Telegram fixture flows
524/525/526 and the production-test telemetry substring classifier527.
Original raw SARIF f4854871cceadb639007dee6a620e165ce058bde098de725cd3baa1c9eb0eeb9
and the four hosted annotations remain evidence. This batch preserves exact
upstream Telegram fixture3c8c27bf6f51a778ed94f452cf6b501f54703b67ef162551f677171916fbcd99.
It changes no scanner workflow, query, exclusion, vendor byte or alert state.

Production-consumer assertions bind the one hardcoded Telegram script URL,
exact permitted data attributes (no auth-url/onauth/post/discussion), and parsed
HTTPS oauth.telegram.org /embed/fixturebot iframe with exact origin/return_to.
Existing wrong-frame, wrong-origin, malformed-schema and replay controls remain.
The actual built-consumer test passes. PM independently exercised18 normal,
adversarial and unsafe-unused-branch/alternate-loader negative controls without
CSP masking. These support only exact test-fixture/product-consumer
non-exploitable classification for524-526; the vendor library is not generally
safe. The served SPA's402 bytes are bound before/after; its manifest claimse672,
whose unchanged Telegram component equals current26f686dab2356cf17431d8370179d46116ec37768aa1617ed71401263fe5eeb1.
This is retained built-consumer evidence, not a fresh current-source build.

The telemetry classifier now parses host/origin, but the observer additionally
retains every cross-origin fetch/XHR. A lookalike, configured-prefix or unrelated
origin allowed elsewhere by CSP cannot disappear from the egress assertion.
Twenty focused controls include collector-level negatives, vendor scripts,
userinfo, schemes and ports; the final assertion requires the configured origin.

Initial browser execution passed Telegram/telemetry-disabled but failed both
enabled cases with zero events. A bounded browser diagnostic captured normal
UA151, webdriver=false, consent=true/DNT=null alongside Client Hints still
advertising HeadlessChrome151. The exact installed SDK bot detector returned
true for those hints and false for consistent human Chrome hints. The fixture
now models that supported browser identity consistently across both surfaces;
production filtering remains byte-unchanged. Original failures and the one
diagnostic-script duplicate-declaration error remain separately retained.
Final combined browser4/0, collector20/0, strict targeted TypeScript and scoped
ESLint --max-warnings=0 all pass.

Frozen14-file handoff w7-codeql-consumer-subset-v2.json
(575ad289948cf114507eb2610fcee99ec78924a6a0988b55ea1a2a27ea9c9927),
archived403528b64db4ccc9ae4ce0c7ed31d0f164e43dc953ec19702037e7bf6559135,
binds exact source, commands/environments, logs, browser results,402 served
files, fixture provenance, prior failed execution and bot diagnostic.
PM independently runs collector20 and accepts the four implementation paths at
P/telemetry-observer-review/completed-v2-review.json
(b2c60b717130da9d1dd54a5b891f7037151998c5ff12fdb8b030abe14e295dee).
This commit contains only those four paths and the worker log. Coordinator owns
any later supported exact-alert disposition and PR push. No zero-finding
scanner result, broader vendor safety, final image/artifact acceptance, merge,
release, signing, blocked mobile probe or remote gate mutation is claimed.

## W9.3 deployment readiness and durable required web approval

The serving process now owns an immutable configured replica roster and startup
generation. It registers before serving, renews its observation, and evaluates
fresh indexed observations against its own generation/roster. Missing,
undeclared, stale, duplicate-generation or unsupported processes fail readiness.
Effective security-feature activation is checked by actual callers, including
inherited/cleared overrides and membership/default changes. Platform operation
policy has a sticky durable activation marker: expiration of a true sibling
cannot restore a false-local legacy path. The admin/operator diagnostic reports
blockers and operation-policy completeness. The drain/roster declaration remains
an operator assertion; observations cannot detect a hidden old serving binary.
Required-web capability is explicitly diagnostic and requires all-replica drain
and upgrade, rather than claiming that a green observation fences old binaries.

No-row users require approval on their first ordinary programmatic request.
New settings default to required=true and300seconds; legacy stored false,
missing preference=false and missing timeout30 remain unchanged. First tool
creation uses the same deliberate new-row timeout. Stored required policy no
longer depends on channel presence. Pending requests precede notification
attempts and remain decidable on web when registration/provider credentials are
absent or delivery fails. Explicit service/org policies, auto-connected
exemptions, session/grant/assistant-mode precedence and live authority remain.
Notification status separates required policy, web capability and configured
provider delivery. Frontend/mobile/CLI preserve unknown and older suspended
server semantics. Approval history exposes Refresh in normal/empty states;
real-query tests cover discovery after empty state, decisions and expiry.

Original source/build failures are retained in w93-prior-compile-failures.json.
The original e95 executable's15-group matrix retains14completed groups with
554pass/4fail/1ignored and an explicitly terminated partial group13
(childSIGTERM-15/wrapper241). Three failures were stale lost-channel preference
assertions; the org-skills failure was diagnosed through the actual generic
approval Pending timeout after independent consent, with the catalog-bound
service identity corrected. A real qualifying auto-provision route control
passed separately; an earlier incorrect filter ran0 and receives no credit.
Historical results are not relabelled as execution of the corrected executable.

The final accepted correction preserves production prefixes and uses explicit
test-fixture preferences to isolate independent authority suites. The actual
new no-settings control uses Nyxmode and the direct proxy handler with fixture
AuthUser: Pending300/no provider/no upstream effect, actual web decision, then
one upstream effect. It is not a new full HTTP auth-extractor test. The final
consistent locked both-KMS backend BuildFinished succeeds in701.790s, copied
binaryb6ca19b94235f2ee50e8b345266f28ad8f252ae8964d68ae5683d4da5018bc52.
All1293 source inputs plus seven reference inputs match before/after/current.
Final eight affected groups execute401distinct tests with401pass/0fail/0ignored,
configured real Mongo and no skip markers. The separately executed new control
passes1 and overlaps that401. The original matrix remains separate earlier
coverage, not a second final-binary whole-matrix claim.

Final scoped locked backend+CLI bins/tests Clippy with bothKMS and warnings
denied exits0 in294.839s (log1eabc02b86030d0d1332a130733b3bcab2875c8f997ed6a4de771f0bf90cb931).
Final CLI BuildFinished produces849b200f445c4527a7f29834ada83b4bf3755e7e122522eb3666479dc94bb0fa;
notification10/0/0 is independently reproduced by PM. The new modern table
branch executes without a captured rendered-text assertion. This is debug
binary evidence, not release embedding proof. Accepted clientv2 frontend66,
types/scopedlint and unchanged ordinary mobile13/types are carried without
redundant reruns; PM real-query5 overlaps that client scope. Original clientv1
legacy/unknown capability copy failures remain retained and superseded by the
reviewed three-path v2 correction.

Frozen backend45 archive19c6fc84ff5aeaedbf3199c7f000d5efb703ea1c643d3fd94532a8843d462e29
and client15 archive0b04c8f6dcefd81d0c17dc1d693e4e96c17385047e8225d90a5ae2ced9af0209
bind the exact reviewed source. Validation w93-final-handoff-v1.json and
w93-final-affected-matrix-v4-audit.json bind source maps, commands/environments,
executable/log hashes, exact group names, prior failures and PM review receipts.
This commit contains exactly50 accepted implementation paths plus this log;
manifest/lock/reference files and unrelated Docker/package/mobile/PM-ledger
work are excluded. No blockedR1-05/06 dependency probe, keyring/OS-service probe,
remote publication, merge, deployment, signing or alert mutation is performed.

Remaining order is the actual hosted updater tagged-digest regression and
metadata-only Rust528 test diagnostic hardening, then W9.4-6 node custody/
migration/separation and W8 durable audit core. Bounded W7 immutable inputs/
producers continue between validations; final exact-export artifact/image,
scanner/SBOM/signature gates remain open. Coordinator owns PR publication and
exact GitHub-generated main3fc integration attribution without merging this
branch. W9.3 acceptance does not close OS custody, external audit witnesses,
final artifacts or the provider-blocked findings.

PM independently copied finalb6ca and passed the exact new no-settings,
corrected org-skills and renamed logout controls3/0/0 with no skips. All45
frozen sources,1293 compiled/current inputs and executable remain unchanged.
P/w93-backend-review/final-selected-v4/completed-review.json
(ab18ffb85c064667d7a11da56aadd32487ba529c05dc2c63217030ce05d437db)
and final-handoff-v1-review.json bind the accepted bounded source/caller batch.
PM returned Mongo idle before this stable commit. These3 overlap final401.

## Hosted tagged-digest source version and bounded Rust diagnostic followup

The actual hosted machine production harness at head574/mergeadf passed its
signed migration and companion parents and real TUF/two image attestations,
then failed rendered bootstrap with source_version_unknown. Docker retained
the official release tag plus digest in Config.Image; source_version passed
the entire tag@sha256 suffix to the strict version parser. The correction
validates the complete lowercase sha256/64hex suffix before using the release
tag. Tag-only references remain strict; digest-only references require a valid
explicit version label. Invalid tags, malformed/repeated pins, wrong repository
and floating versions cannot fall back to a label. Downgrade refusal and explicit
rollback remain. The exact tagged-digest production fixture/pin is unchanged.

The reviewed9d0c patch applies byte-for-byte. Locked updater test BuildFinished
completes in26.336s, producing copied debug executable
589efbda0d47a65ca60e695522ba0dfafd999946630c781feb3f88c292803299.
Selected docker-module3 and mocked-controller12 pass with0failed/0ignored.
New bootstrap/replace cases reach deliberately refused verification for valid
references and refuse malformed/downgrade inputs before Docker mutation or a
replacement journal; existing local progress-file writes are allowed. These
tests use local mock HTTP, not a successful signed container migration.
Scoped locked updater bins/tests Clippy with warnings denied exits0 in13.593s,
log19065f58c6d04940eabb200530444fcb8aa9d7600fe19c416e6761f77cf5e515.

Frozen w7-updater-tagged-subset-v2.json
(306317d7e63b31206b26496d19c9456bc79846472e7af310364012716f447964)
and archive ddef9af7111178a07ad46717a4577f2c46684bbee0f00a636e18813e584dcc74
bind50 unique selected/transitive inputs, six command/log pairs and the copied
executable. Original build/runtime/Clippy maps contain45; five additional
shared TLS/root/machine resource references match stable0b35. Final locked
no-run6.857s binds all50 before/after and produces the byte-identical executable.
Cargo reports fresh=false after Clippy; cache reuse is not claimed. Two failed
evidence-auditor assumptions (fresh=true and52 unique paths when two were
duplicates) are retained/corrected in the linked preflight receipt. Original
v1 archive/receipts remain separate. PM independently copied589e and passes
the exact new parser/controller2/0/0, overlapping the worker15, with original
PM relative/absolute-path preflight failure retained before its corrected run.

Rust CodeQL528's test-only diagnostic drops whole Result Debug formatting in
favor of fixed failure text. approve returns AppResult<()>; this is bounded
error-detail hardening, not evidence of a raw-key leak. The Conflict predicate,
credential/key counts and Pending rollback assertions remain unchanged. Exact
reviewed652b patch and rustfmt/diff checks bind the static change; no redundant
backend rebuild is attributed to it. Timestamp-only device expiry UX remains.

The original hosted bootstrap failure and its preceding successful checks are
retained separately. Real rendered production bootstrap, upstream3fc integrated
source, final images/artifacts/scans and the remaining W7/W8/W9 gates stay open.
Coordinator owns normal PR publication and exact-alert disposition. This batch
contains only the two updater paths, one test diagnostic and this worker log;
unrelated Docker/package/mobile/PM-ledger work is excluded.

## Authenticated rustup installer mode dispatch

The actual hosted ARM release-cache job117929205830 at merge076735af passes
the corrected exact-source export, then fails invoking the authenticated
rustup1.28.2 installer as a file named download: unknown proxy name 'download'.
The original loge3702ad5550c7e1c8acdb28336b54e7f81d983a3aa033be44b47ccd9d21996f0
is retained. The correction selects basename rustup-init only for that tool.
Other archives retain download. The committed catalog, URL/digest, host target,
HTTPS/download bounds, integrity verification and install arguments are unchanged.

Two durable regressions verify successful verification before execution, the
installer basename/mode/bytes and exact intended install arguments, and refusal
without execution plus staging cleanup on verification failure. Full build-input
policy/unit tests pass17/0 under CARGO_TERM_COLOR=always/RUSTUP_TOOLCHAIN=1.98.1.
The initial17-pass run has nine selected maps; the final run separately binds
all3087 selected copied policy-corpus/manifest/helper/native inputs unchanged.
Neither includes a mobile dependency probe or an actual toolchain installation.

The exact pinned native aarch64-apple-darwin installer20ef5516c31b1ac2290084199ba77dbbcaa1406c45c1d978ca68558ef5964ef5
independently reproduces the old download --help failure(exit1), while identical
bytes named rustup-init --help succeed(exit0). A separate actual production-main
harness runs the real integrity verifier over those authenticated bytes, asserts
the unchanged intended install argv, and substitutes only --help for native mode
dispatch. It checks absent scratch Cargo/Rustup homes and cleaned staging. This
is native macOS help-mode execution and captured intended installation arguments,
not Linux/ARM execution, toolchain installation or completed release-cache proof.

Frozen w7-rustup-basename-subset-v1.json
(7806f2e4b4a5e892115c97489ccc23c2f39b27169d655b14c3c8a5bcb6882207)
and archivef7519bbee57191e2a8757c3a2178688f5701ea3dd758416ec4f8a1954984e5cb
bind exactly two changed scripts and seven unchanged references, four command/log
pairs, the native source record and helper hashes. Original hosted failure and
the original basename negative remain distinct. Node foundation WIP, unrelated
Docker/package/mobile edits and PM-owned ledgers are excluded from this batch.
Coordinator owns normal draft-PR publication and subsequent hosted attribution;
no worker push, merge, deployment, release publication or remote alert mutation.

### W9.3 billing caller fixture followup (2026-10-19)

- Preserved actual hosted billing-smoke failure at published a577 / generated
  076735af: approval_failed after 300 seconds, 0 passed / 1 failed, 301.32s
  harness duration. Its shared response helper alone does not identify ingress.
- Read-only Pending metadata from the retained original b6ca executable showed
  access-token POST /chat/completions for chrono-llm-public, lazy required=true,
  no explicit service override, no delivery channel, and no usage effect.
  Sequential first-effect/source tracing binds that diagnostic to the mounted
  assistant direct completions caller. The bounded child was terminated after
  observation; this incomplete diagnostic is not a smoke runtime verdict.
- Preserved separate actual original buffered recovery failure: 0/1 in 5.63s
  test-reported time, downstream wait elapsed, Pending GET /blocked with a
  300-second approval timeout and zero usage rows. No product bypass was added.
- Corrected only billing_integration_tests.rs. The smoke uses one unique owner
  settings row false -> true for the existing exact approval/redemption block
  -> false afterward. Readback asserts one row and the effective preference.
  The buffered settlement fixture explicitly sets false for its ordinary route.
  Original exact request/digest/redemption/settled-count assertions and the
  downstream settlement recovery block remain byte-identical; insert_owner,
  production default-required behavior and all production sources are unchanged.
- Built an isolated detached committed ee0 snapshot plus this reviewed candidate,
  excluding mutable node transaction work. Locked backend test build with both
  KMS features completed with actual BuildFinished/artifact in 575.061s. Copied
  c178e805b9c2 executable is bound to 4,277 unchanged selected tracked regular
  files, not an exhaustive compiler environment or build-hook graph.
- Actual copied-executable full smoke and exact buffered cases passed 2/0/0,
  one thread, configured Mongo, no skip markers and no new database left behind.
  Per-command runner durations were 12.076s / 0.993s; the Rust test summaries
  reported 5.33s / 0.94s, each with 9,388 filtered cases. Mongo returned idle.
- Locked scoped nyxid-server bins/tests Clippy with both KMS features and
  -D warnings passed in 285.151s. All four command before/after/current input
  maps and the copied executable remained equal. Applied the byte-identical
  validated candidate to the main worktree after isolated validation.
- Frozen eight-input handoff w93-billing-final-subset-v1.json SHA256
  6401eda51308b9886af239ec9fe0905c7b560c3803bf41e0edd122ee9b176905 binds
  the archive, four command/log pairs, original diagnostics, copied executable,
  PM candidate/runtime/Clippy receipts and unchanged references. Earlier failed
  diagnostic preflights remain retained separately; no hosted rerun or final
  artifact, node transaction, W8, merge, deployment or release claim is made.

### Hosted076 backend caller-policy fixture correction (2026-10-19)

The actual a577/generated076 backend and coverage jobs were cancelled at90m.
Their partial raw results remain separate: nextest2838PASS/10FAIL/38TIMEOUT,
9473scheduled+11skipped; coverage4383ok/52FAILED/two observed ignored, without
final JUnit/summary. The exact union54 names is retained; two billing cases
were already corrected by d289. Focused original52 diagnostics first captured
50 actual Pending300, one security_readiness_unavailable12900, and one25s
incomplete case. A longer bounded observation captured that case's Pending.
Terminated diagnostic children are incomplete observations, never pass results.
API-key requester IDs are credential IDs: requester_id_equals_policy_owner
alone does not establish actor equality; source/auth binds each actual caller.

Only20 selected test/helper paths change. Explicit approval preferences are
set for evidenced execution actors/effective owners, including later org and
non-admin branches. The shared helper updates the actual settings row and
asserts effective readback; generic fixture/user/database defaults do not opt
out. Targeted org-team setup uses real test readiness/roster/heartbeat, retaining
missing/old-roster controls. Production prefixes and default-required policy
remain byte-identical. No timeout increase or production bypass was added.

REST denial retains its allowed POST prelude and both blocked GET assertions.
MCP denial retains explicit per-service true, denied zero-persistence and the
allowed branch's one actual persisted approval. It joins an actual web reject,
asserts rejected/web state, HTTP200 and parsed isError/rejection text. Exact
redemption, no-settings assistant dispatch, required/reconnect, auto-provision,
org policy and live scope/downstream/body/wire-log assertions remain covered.

Validation uses an isolated detached exact GitHub-generated076 checkout plus
the d289 billing test delta and candidate20; node and unrelated WIP are absent.
Natural first build101/232.509s with two unqualified Json test errors remains;
only those two sites were qualified. Intermediate4c selected54 results remain
53pass/one natural301.743s failure from a later independent non-admin caller.
The exact source/actor/defaulttrue observation justifies its one-line explicit
preference. The remaining27 ran unchanged4c before that correction. Historical
4c and focused diagnostics are not combined into a final passing matrix.

Consistent final locked both-KMS no-run build passes644.220s with actual
BuildFinished/compiler artifact, copied6313da949a09933e650d38bd5b81726376566dbc1fe629d3c9666373f8b84c4e.
All4,281 recorded input path entries match before/after/current for each
successful command; two existing documentation symlink aliases yield4,279
resolved targets. These are not an exhaustive compiler/build-hook identity
or environment tree. Selected20 main/isolated source hashes match exactly.
Rustfmt check passes; locked scoped nyxid-server bin/tests Clippy with both
KMS features and -D warnings passes289.480s. All28 successful command/log
pairs are retained (build, format, Clippy, exact27,16groups,eight controls).

Final copied-executable exact27 passes1/0/0, overlapping full group9. Actual
16 full groups contain515 distinct top-level names:512pass/0fail/three existing
ignored. Eight disjoint required/default/org/auto-provision approval controls
pass8/0/0:520 distinct passes total. All original54 map to actual passing final
group names. Every runtime test command enforces exact names/cardinality/ignored parity and
no skip diagnostics, with unchanged binary/maps and Mongo returned idle.
Two tools-manager local CLI E2E ignored entries are not executed by this batch.
The ignored readiness child is explicitly invoked twice by its passing parent;
those child invocations overlap the parent and are not additional coverage.

The proposed local commit is exactly20 test sources plus this log on d289.
Only selected20 blobs are bound to that tip; the entire4281 integrated snapshot
is detached076, not the local branch or final release embedding. Original
hosted failures, compile failure and4c natural failure remain retained. Node
transaction/runtime/OS/Linux, W8, full new hosted CI, final artifacts/scans and
publication gates remain open. Coordinator owns normal draft-PR publication;
no worker push, merge/rebase, deploy, key-service or guest action occurred.

## W7 Focal cargo-auditable host-tool compatibility correction

Actual generated229 (parents ad233 + 6acab) ARM bootstrap passed source export,
rustup/toolchain installation and verified cargo-dist, then the pinned GNU
cargo-auditable failed loading GLIBC_2.32/2.33/2.34 in the pinned Focal image.
Original raw log118014928748 and its authentication/catalog inputs are retained.

The bounded two-script correction selects the existing pinned same-architecture
MUSL cargo-auditable asset for Linux GNU hosts. Host detection, Rust build target,
other tools, catalog pins, HTTPS/download bounds and checksum verification remain.
Tests exercise both Linux architectures and retain other-tool target selection.

Executed native ARM64 offline Focal controls through the actual verifier/main:
original authenticated GNU loader refusal; authenticated static MUSL dispatch
success; tampered static archive rejected before execution with empty destination.
The MUSL ELF has no PT_INTERP or GLIBC version references. The mounted authenticated
cargo remains read-only. Its printed1.98.1 version is cargo passthrough; the
cargo-auditable0.7.7 identity comes from the pinned verified archive.

Final Focal runner exit0/0.766s and complete build-input unit suite18/0,
runner18.779s, are bound in w7-auditable-static-handoff-v1.json. All3,087 recorded
unit input entries match before/after and current before this log-only append.
Earlier harness failures remain: v1 omitted cargo; v2 wrongly expected auditable
version text from cargo passthrough. The first unit pass used only a limited
three-entry input list; the final18-test command uses the full3,087-entry map.

This establishes native ARM64 Focal loader/verification/dispatch compatibility.
Application cargo build, embedded metadata, amd64 execution, final release
artifacts and hosted integration remain separate open gates. Node WIP excluded.

## Main c171 shared health fixture compatibility

Main c171 independently added the same typed channel-bot health fixture at the
local insertion point. The candidate aligns that shared block with upstream and
keeps the narrower effective channel-bot route scope through an early guard.
Unrelated health paths retain their original empty object; conversation health
paths retain their original empty conversations response. No production code or
policy fixture is changed.

A one-file temporary commit on a8bed merges with c171 using merge-tree exit0.
The preview fixture equals the candidate. Its merged X spec retains the accepted
production-policy import and all three upstream health-unavailable assertions.
No branch, real index or working-source merge was performed.

On that isolated preview, three actual Zod health parses and17 exact original
route/fallback comparisons pass. Scoped ESLint passes. All32 listed tests in the
complete X, WhatsApp and standalone setup browser files pass with zero failures,
skips or retries. Four command/log pairs have identical4,309 recorded input path
maps before, after and current. These are Vite Chromium fixture integration
checks; production CSP/build and full frontend/backend acceptance remain separate.

PM separately bound upstream scheduler heap-pinning changes to actual native
generated43, whose entire tree equals c171: selected scheduler passes in both
backend and coverage; full counts9481/0/8ignored and9497/0/10startup skips remain
attributed to upstream. Original generated229 abort and external diagnostic plan
are retained; the unused diagnostic is retired without execution. The twelve
other fresh generated229 backend failures remain open and distinct from old54.

Evidence: w9-health-fixture-c171-v1/final-handoff.json. The local commit contains
only this shared fixture and log on a8bed; broader W9 and unrelated WIP excluded.

## Batch 0 — replacement-worker triage, 2026-10-19

PM/reviewer: heca 01a15431 (Claude Opus 5.5); worker: Codex GPT-6.1-sol, xhigh.
HEAD be149b7c9; origin/main 618c627b9 is five commits ahead; no implementation edits or commits.
Dedicated target/logs: /tmp/nyxid-w5w9-codex-b0.SPJwrX; own Mongo 8.0.17 replica set on port 50002, commit/abort controls 2/0.
Executed cargo test --locked -p nyxid-cli: 1768 passed/0 failed/7 ignored, including 78 node-preparation cases.
Executed cargo check --locked -p nyxid-cli --features node-proxy-test --lib: exit 0; cargo clippy --locked -p nyxid-cli --all-targets -- -D warnings: exit 101 (unfinished-node dead code and ordinary lint findings).
Mobile frozen pnpm 10.34.7 install with repository .npmrc and --ignore-scripts passes; security patches 8/0/0, typecheck exit 0, application tests 52/0/0.
Earlier scratch runs omitted .npmrc (0/8 module-resolution failures) and the backend alphabet fixture (51/1); corrected copies pass without source/test changes.
Executed python3 scripts/test-build-inputs.py: 18/0; check-build-inputs.py, cargo fmt --all -- --check, git diff --check, four shell syntax checks and Python AST check pass.
Archived adapter git apply --check fails at secret_backend.rs; v3 plan is reference only. Registration prep builds; legacy readers/writers, migration, daemon quiescence and OAuth durability/logging remain draft.
APT wiring is partial; APK catalogs/wiring and Linux image/install tests remain open. Planning docs retained; ledger ownership/status refreshed. Awaiting PM Batch 0 verdict before merge.

## Batch 1 — main integration, 2026-10-19–20 (awaiting PM review)

PM/reviewer heca 01a15431 (Claude Opus 5.5); worker GPT-6.1-sol/xhigh. W8 belongs to stream B heca 01a154bb; its owned sources were untouched.
Mobile 5cdca82b0 confirms GHSA-86w9-cpqp-85rv/CVE-2026-85393 and cites GHSA; post-edit patch tests 8/0/0. Real merge 31b3e4e0f integrates origin/main 618c627b9 without conflicts.
Fixture commits 192daae96, 8d7e1fced, c1ea89810 declare exact owner/service preferences; global required=true and production approval behavior are preserved.
Captured FAQ403/api_key_scope_forbidden/code9000; gateway personal actor/owner Pending then15s Elapsed; Discord admin-catalog target Pending then30s drain Elapsed despite a separate personal auto row.
New guest missing/required-policy controls preserve403/code9000, zero forwarding and zero approval/grant/ack effects; original revocation/denial assertions remain.
Both-KMS selected executable:45 passed/0 failed/0 ignored. Full same-artifact retry:9509 passed/6 failed/11 existing ignored,24959.29s,exit101; all original12, new guest control and scheduler regression pass in both.
Six extra full failures: no-progress4s bound, onboard1s audit wait, X reply polling, near-expiry DeviceCodeExpired, renewal2s budget, eight-owner90s budget. Exact names/panics are in SECURITY_W5_W9_REVIEW.md; no assertion/timeout/ignore changed.
Unchanged diagnostic onboard and X reruns each1/0/0; no-progress diagnostic reruns3x fail only the total-time bound after error/stage/response-ID assertions pass. These do not replace the failed full result.
Original cargo full attempt was interrupted after491passes/20failures when disposable Mongo aborted on error24/nofile8192. Fresh PRIMARY used nofile60000/2GiB cache; redundant Cargo rebuild stopped and exact both-KMS test executable reused, not a clean cargo-test exit.
Executed cargo test -p nyxid-cli with package test debug=0/codegen-units=256 and four threads:1768 passed/0 failed/7 ignored.
Frontend npm run build exits0; npm test -- --maxWorkers=1:6255/0/0 in586 files; npm run lint:0errors/33warnings. Native nginx microphone/worklet and enforcing-CSP negatives:2/0/0; final pinned-image/credential-page parity remains open.
Backend all-target both-KMS Clippy exits101 on four CLI WIP needless_borrow diagnostics; supplementary backend-bin Clippy exits0. CLI all-target Clippy exits101:18 dead_code+4 needless_borrow; untouched for Batch2. Format/diff checks pass.
All23 non-document node/package WIP paths retain saved bytes/modes except main's expected Dockerfile COPY line; none committed. Logs/target:/tmp/nyxid-w5w9-codex-b0.SPJwrX. No push/publication; Batch1 verdict pending before any Batch2 commit.

## Writes-only approvals — 2026-10-20 (awaiting PM review)

Feature2e41f8efc; API/design/rollout docs9e8e7805f. Requested worker-log-only rewrite isf3cdae70b; the two ledgers remain untracked.
Absent preference is Writes only; stored true/false retain All/Off. Lazy creation preserves absence; unmarked unreleased W9.3 true rows retain true because owner choice cannot be distinguished.
HTTP/MCP use trusted effects, unknown is Write; inference uses the existing closed stateless catalog contract. Internal admitted NyxAgent transport is Read; callbacks retain independent checks. SSH/machine, Deny, Session, org, scheduled/durable controls remain.
Web/mobile/CLI expose Writes only (default), All requests, Off; legacy boolean writes retain All/Off semantics. No per-service fixture exemptions were removed.
Both-KMS cargo test --no-run exits0; copied executable --exact selected names --test-threads=2:573/0/0 in2963.38s, including every original54 and Batch1 twelve. Nine first-pass cases overlap; no Mongo skip diagnostics.
Three detached readiness-heartbeat teardown tasks panic at security_readiness_service.rs:350 on Mongo215/database-being-dropped; originating tests pass. These diagnostics remain separate from harness totals.
All six prior timing failures rerun serially3x on the original binary:18/0/0; hosted Linux run39346438958 passes all six. PM closed their investigation. Original full9509/6/11 remains recorded; no bounds/assertions/ignores changed.
CLI cargo test -p nyxid-cli:1778/0/7 existing ignored. Frontend npm test -- --maxWorkers=1:6258/0/0; npm run build exits0; npm run lint:0errors/33existing warnings.
Mobile npm test:52/0/0; npm run typecheck exits0. Production backend-bin both-KMS Clippy exits0; dirty-tree all-target Clippy finds only node-WIP lints, left untouched.
Committed-only CI fix32cddd4f8 adds the CLI test-library encryption namespace re-export; unrelated node WIP is excluded. Exact CI all-target/workspace/both-KMS Clippy passes on committed1b8722613 plus the fix: exit0, zero diagnostics,5531.78s on macOS.
CLAUDE.md/docs/ENV.md patch is prepared but unapplied pending PM shared-file permission. Logs/target:/tmp/nyxid-w5w9-codex-b0.SPJwrX. No Batch2 code or publication.

## Writes-only hosted follow-up — 2026-10-20 (awaiting PM review)

Method-override guard and real-Mongo rejection control4fa83f0e9; stale proxy/inference API wording8f68142b6. Only explicit owned pathspecs committed.
Frozen backend163cf7625 plus the correction builds both-KMS; current correction bytes match its source. Native Apple ld emits the existing compact-unwind size warning; build exits0.
Exact selected binary run124/0/0 in863.98s includes all original54 and Batch1 twelve; zero readiness teardown panics.
Hosted-three normal run: invocation1/0/0, capacity recovery5/0/0, capacity heartbeat5/0/0; zero Pending approvals/observer errors. Heartbeat captures6–7 persisted timestamps; original slot/retry assertions pass.
Controlled explicit-All injection reproduces invocation running!=completed at:3788; Pending access_token POST /v1/responses, read verb, exact fixture owner/service. This intentional diagnostic fails1 test; unset writes-only completes with no settings/approval rows.
Capacity fixtures retain Session auth; both test bodies and runtime service match hosted9ec8f8599. The final slow-body first_byte_timeout is an original expected assertion. No bounds/assertions/ignores changed.
Own isolated all-target Clippy stopped at PM shared-build-policy change; no result claimed. LLVM build uses shared target/jobs4/incremental0/dev-debug0/test-debug0, no profile overrides, RUSTFLAGS=-Dwarnings; exit0 with the native Apple compact-unwind warning (linker_messages explicitly ignores -Dwarnings).
Instrumented invocation via cargo llvm-cov test --locked -p nyxid --no-report passes1/0/0. Reusing its copied instrumented binary --exact --test-threads=1 gives capacity recovery3/0/0 and heartbeat3/0/0; six nonzero llvm-profdata profiles verified.
Instrumented recovery durations85.96/93.10/71.30s; heartbeat61.63/61.49/61.70s. Zero Pending/observer errors; heartbeat6–7 timestamps. Historical coverage timing cause remains unproven; hosted Linux decides full concurrency.
Two harness CLI preflights rejected arguments before compilation/tests; the next queued Cargo run was stopped without building, then the successful instrumented artifact was reused. CLAUDE/ENV patch awaits shared-file permission. No Batch2 code or push.
PM hosted resolution: run39362062126/head8589cfd8 passes Coverage (Backend) and Backend Test, including writes-only invocation. capacity_wait fails only Coverage (Backend Base) on mainaaa3d772; PM closes the capacity investigation as an upstream baseline flake.

## W9.5 checkpoint 1 — custody evidence v4, 2026-10-20 (review pending)

PM accepted8216ce309; this checkpoint adds optional credential_custody and requires Some(true) in otherwise supported evidence, with fixed node_custody_unverified guidance.
Evidence version4 rejects v3/unknown reports; explicit shared_legacy selection remains usable. Positive fixtures report actual expected custody explicitly; no production true default.
Shared-policy cargo test --locked -p nyxid-machine context:::15 passed/0 failed/0 ignored (combined working tree includes D's one pending container-diagnostic test); log ~/.cache/nyxid-stream-a-evidence/w95-protocol-tests.log.
Only custody/v4 protocol and three fixture hunks are staged; D's browser diagnostic field/reason/tests and all runtime/probe WIP remain uncommitted. Owned rustfmt/diff checks pass.
W9.5 remains open: runtime/native denial and final Clippy gates pending; Linux production CLI built, but guest test compile was SIGKILLed before tests and the VM remains stopped under PM memory/benchmark hold.
Approved backend existing-assignment guards/necessary discovery consumers follow after C's announced edit window; stored modes/capabilities must not change. EXECUTION/REVIEW remain local/untracked. No push.

## W9.5 backend guard validation — 2026-10-21 (review pending; CLI/native tests open)

PM identified86ee63b25 being committed before Clippy; history stays unchanged and no further A commit precedes applicable gates.
Original machine all-target Clippy completes with zero diagnostics (successful Cargo log survives; command session/independent exit record was lost after restart).
C combined cargo clippy --locked -p nyxid --all-targets --features aws-kms,gcp-kms --no-deps -- -D warnings exits0/zero diagnostics at00:48:35; actual TEST driver29823 starts00:48:49 after A23:14:28 freeze. Both-KMS test-no-run exits0 at02:04:30.
Preserved nyxid-server-combined-tests machine_access --test-threads=4 --nocapture exits0:46 passed/0 failed/0 ignored in116.91s, including both new current-custody/support regressions and44 existing matches, on real replica-set Mongo50044 with isolated test databases.
Final output recorded02:08:11.490025+08; A idle confirmed02:08:44.974550+08. PM/C/B notified; no further direct tests, Cargo, Lima or Docker during B quiet window. CLI remains unqueued until B clears and C successor actual TEST starts.
Backend source freeze unchanged; current support derives from authority v2, current complete v4 evidence and the node's advertisement. Admission refuses unsupported separated assignments; discovery retains them with a fixed reason and no executable tools except cancellation. Stored modes, capabilities and reports stay unchanged; shared_legacy remains usable.
Original CLI Clippy exits101 on new-test browser bool/Option mismatch and custody collapsible_if; fixed directly without changing assertions. Corrected exact all-target/node-proxy-test CLI Clippy exits0/zero diagnostics at04:52:34.199259 after the authorized stop/cancel correction; all22 freeze paths verify with only recorded replacements. D standalone machine65/0/0 and all-target Clippy exit0.
This backend checkpoint commits only four owned backend paths and this log. CLI source stays uncommitted until targeted tests run after C's next actual TEST-start signal. Lima/Docker stay stopped under PM memory hold; native Linux custody remains a separate unexecuted evidence gate, with no isolation closure claim.
Logs persist under ~/.cache/nyxid-stream-a-evidence/w95-{backend-machine-access-combined,corrected-cli-clippy,corrected-cli-drivers}.log and C gates/post-reboot-combined-*. EXECUTION/REVIEW stay local/untracked; no push.

## W9.5 CLI custody/runtime/session checkpoint — 2026-10-21 (review pending; Linux evidence open)

PM accepted f22750504. Real dropped-identity opens include selected/sibling keyfiles; keychain challenges a public synthetic sentinel in the supervisor's native service. Only typed permission denial proves custody; success/unknown remain distinct, with no same-UID isolation claim or real credential retrieval.
Per-identity/context Arc caches observe construction/identity changes and refresh every60s, expiring after100s; execution reads cached state without a probe syscall. Machine status exposes fixed per-role metadata and invalidates stale/missing claims.
Actual shared_legacy jobs still run with downgraded claims and the explicit token/signing-secret/stored-credential warning. Separated effectful execution refuses unknown/failed custody with12419; JobCancel, Cancel, CancelExact and DesktopClose remain available with unchanged signature, authority and target-scope checks.
Hidden shell-free helper dispatch precedes telemetry/update/skill hooks. Explicit native daemon install/reinstall binds macOS Aqua or the Linux supervisor's %t/bus and dbus.socket ordering; start never re-renders, both old/new definitions validate, and file fallback is explicit.
Corrected all-target/node-proxy-test CLI Clippy exits0/zero diagnostics04:52:34; the original E0308 fixture type and collapsible_if are corrected without assertion changes. Exact shared5env used throughout, with one prepared test-no-run queued only after C's verified next TEST-start06:08:16.
Fresh CLI TEST driver78132 starts07:18:22; no-run exits0 at07:20:07.897700. All22 original source hashes plus authorized replacements/desktop helper verify before launch and after tests; no additional Clippy or duplicate Cargo.
Eight disjoint selectors finish07:23:07.461774: custody8/0/0, migration25/0/0, daemon50/0/0, keychain5/0/0, authority_runtime13/0/0, status2/0/0, separation8/0/0, setup15/0/0; total126/0/0. Runtime controls exercise actual legacy jobs, all4signed stops, tampered signatures, cross-scope denial and effectful refusal.
Migration retains complete bidirectional/failpoint coverage; no macOS keychain behavior difference is observed after C's zbus switch. Native Linux dropped-identity/key-service and no-network/no-file acceptance remains unexecuted; Lima/Docker stay stopped under PM memory hold, with no W9.5 closure claim.
Persistent logs, source/result metadata and the preserved fresh test executable live under ~/.cache/nyxid-stream-a-evidence/w95-corrected-*. Only A-owned paths/hunks are committed after ID releases the index; D browser/diagnostic/prose WIP remains untouched. EXECUTION/REVIEW stay local/untracked; no push.

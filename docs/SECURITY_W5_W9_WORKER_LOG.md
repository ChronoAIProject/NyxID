# W5–W9 implementation worker log

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

## Remaining authorized batches

W5 final container review/integration; W7 pinned,
locked and scanned builds, shipped-artifact SBOMs and mandatory official manifest
signing; W9 all new-key creation defaults, readiness, real OS identity/key-service
boundaries and resumable full secret migration; W8 strict attribution, complete
critical-operation durability, independent protected witness, prefix retention,
key lifecycle and scoped API/UI. Major W8 code follows a concrete operation and
architecture proposal sent to PM. No scope row is closed by this log.

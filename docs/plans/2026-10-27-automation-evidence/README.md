# Automation #2040 validation evidence

These files support the owning Astra [sign-off](../2026-10-27-automation-signoff.md)
and [review ledger](../2026-10-27-automation-review.md). They contain local fixture
results and source/binary identities. No test binary, Mongo data or credentials are
included. Both passing and failed attempts are retained.

`commands-2bf59012.json` preserves the actual first successful build and four direct
binary commands from the existing Astra Heca session. The matching
`automation-2bf59012-validation.json` contains all 107 executions / 106 unique test names,
one duplicate, original log hashes and the historical binary SHA. Its42-entry source
manifest matches commit 2bf59012. Those passes are carried forward for unchanged code;
they are not relabeled as executions of a later binary.

The final correction test/Clippy logs have sibling `.json` records containing exact
commands, isolated Mongo/v2 environment, FD limit, actual Cargo/compiler argv, owned
process group, disk-guard threshold/minimum and exit status. The final scheduler
batch executes only the 16 cases omitted by the original filters. Its child exited0
with 16 passed / 0 failed / 0 ignored, but the first wrapper verifier returned 126 because
one timing print split a test name from its `ok`. Original `.exit`/`.json` remain
unchanged; `.verified.json` records the corrected parser result. Astra independently
checked all 16 exact results. This was not a product-test failure or rerun.

`cargo-resource-profile.toml` is an exact copy of the coordinator-owned resource
configuration used for the recorded commands. It preserves dependency settings,
assertions, overflow behavior, optimization and test coverage. `rustc-version.txt`
records the local toolchain. Final formatting and whitespace checks have their own
command/exit/timestamp records.

The fake NyxAgent publisher fixture uses local endpoints and performs local readback.
It establishes the approved #2040 completion behavior; no production publication is
required or claimed. Applicable combined I-01/I-02/I-03/I-04/I-06 remain separate gates
until the coordinator's exact integration candidate is built and exercised.


`final-verification.json` records Astra's exact-source review and local-validation
completion for `6df83d87b826141ec025ce2276f66cb147cbfe33`, the final 42-entry source
manifest, corrected binary identity, all 134 unique successful backend cases across
136 executions, final Clippy pass and open combined gates. `SHA256SUMS` fingerprints
every file in this evidence directory except itself. The implementation commit is
separate from the later documentation-only evidence commit.


Raw `.log` files preserve exact process-output bytes, including terminal blank lines
and occasional trailing spaces. An archive-wide whitespace check reports those original
bytes; they are deliberately retained so log hashes remain verifiable. Source and report
whitespace checks pass separately. No source formatting rule or lint was disabled.

# Nonpublishing OIDC signing rehearsal

`security-release-rehearsal.yml` runs as a required CI dependency after actual
artifact packaging on relevant pull requests, and weekly with the same packaging
jobs. It signs the actual staged ten platform images/five indices and four CLI
archives, including their final-byte SBOM membership. It explicitly records that
PR coverage does not include global release installers. No source manifest alone
is treated as a shipped-artifact inventory.

The rehearsal has its own exact Fulcio workflow identity:

```text
https://github.com/ChronoAIProject/NyxID/.github/workflows/security-release-rehearsal.yml@<RUN_REF>
```

For PRs the ref is `refs/pull/<NUMBER>/merge`; schedules use `refs/heads/main`.
The reusable workflow identity, rather than the calling CI workflow, is the
signer. Its issuer is `https://token.actions.githubusercontent.com`. The production
updaters' trust policy is unchanged. After successful exact-W6 verification,
`check-rehearsal-rejection.py` verifies that the authentic bundle fails specifically
at the workflow-identity check for both production workflows, including at the
same ref and at their production tag/main refs. A failure for an unrelated reason
is not accepted as identity-separation evidence.

Only the rehearsal job receives `id-token: write`; other PR jobs use read scopes.
A reusable caller must grant that permission ceiling to this job. It needs no
repository secret, production environment, `attestations: write`, `contents:
write`, `packages: write`, registry login or stored key. The signer uses the
isolated `security/signing-tools` npm lock, installed by checksummed npm with
`npm ci --ignore-scripts`. All dependency archives require registry SHA-512
integrity and zero install hooks. The reviewed producer uses Sigstore sign/bundle
3.0.0 and protobuf 0.3.2 to preserve W6's exact Rekor DSSE serialization, including
the empty `keyid` field. Its core override is 3.2.1, fixing the DSSE type-binding
advisory GHSA-jfc7-64v2-mr8c. Offline tests exercise actual DSSE signatures,
tampering, payload-type binding, serialization and OIDC refusal before the live
rehearsal. Dependabot proposes weekly updates; lock integrity and unchanged-W6
compatibility must be reviewed together. Every discovered lock, including this
one, runs through the dependency audit. The workflow never disables transparency
or TLS/certificate verification.

The actual external effects are:

| Effect | Destination and retained evidence |
| --- | --- |
| OIDC token request | GitHub's job token endpoint. The ambient token stays in the signer process and is not written to artifacts or logs. |
| Short-lived code-signing certificate | Public Fulcio issuance and certificate transparency. The workflow identity is public. |
| DSSE transparency entry | Public Rekor, containing the signed statement, named artifact/manifest digests, and certificate. |
| Temporary build/signing evidence | GitHub Actions artifacts with seven-day retention, ordinary workflow logs and check statuses. |

It does **not** call GitHub's repository attestation-upload API, create a release
or tag, upload an application image or OCI referrer, or promote a registry tag.
The public certificate/transparency writes above are necessary signing evidence;
application artifacts remain temporary CI evidence. No production key or remote
configuration is provisioned by implementing or locally testing this workflow.
The PM pushes the reviewed change and hosted PR execution supplies the real OIDC
evidence.

`Release Signing Policy / W6 Signing Compatibility (amd64/arm64)` runs the exact
included consumer's authentic historical fixtures and cryptographic negatives.
`Signing Rehearsal / OIDC Rehearsal and Production Rejection` supplies the live
OIDC/proof/identity-separation evidence. Both gate `CI Pipeline`. A missing OIDC
permission, failed signature, unavailable TUF/transparency service, incompatible
bundle or accepted production identity fails the check. The workflow cannot
silently skip a required signing step or replace failure with a warning.

Production custody and administrator prerequisites are in
[RELEASE_SIGNING.md](RELEASE_SIGNING.md). A rehearsal signature cannot authorize
an official publication, even when source bytes and digests are identical.

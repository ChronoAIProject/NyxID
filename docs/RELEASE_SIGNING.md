# Official release signing and verification

Both official publishers require a keyless Sigstore signature over
`official-release-manifest.json` and every subject it describes before publishing
application bytes. There is no signing-disable switch, arbitrary signing command,
unsigned fallback, or verification opt-out. Missing authority configuration,
signer output, verifier, authenticated trust metadata, or valid signing evidence
fails the job. A successful signing Action alone does not authorize publication.

## What is signed

`scripts/official_release.py` derives the manifest from the independently checked
publication plan and final artifact evidence described in [RELEASE_SBOMS.md](RELEASE_SBOMS.md).
The manifest records the exact source commit/epoch, producer scope, workflow ref,
OIDC issuer/identity, named SHA-256 subjects, and supporting SBOM digests. Its own
SHA-256 is also a signed subject. The generated `subjects.sha256` is the complete
input to the pinned provenance Action; the verifier rejects changed, duplicate,
extra, missing or renamed subjects and mismatched provenance source/workflow.
Signatures remain outside all sealed application asset trees.

| Publisher boundary | Mandatory evidence before the first write |
| --- | --- |
| CLI `dist host` | Complete planned archives/installers/checksums/source, actual binary/archive SBOMs, the standalone credential manifest and its fingerprint, and the release SBOM closure receipt. |
| CLI GitHub Release creation | Recompute that complete manifest, repeat cryptographic verification, and require the exact upload file set, including the verified manifest and bundle. Unplanned files cannot enter the upload glob. |
| Platform image copy by digest | Actual scanned OCI subject, passing Trivy report, actual filesystem/binary/frontend SBOM composition, and a signed platform manifest. |
| Multiarchitecture index write by digest | Exact runnable platform descriptors and composition SBOMs, and a signed index manifest. |
| Image tag promotion | All ten native platform subjects and five indices in the independent plan, complete SBOM closure, and a signed complete image-release manifest. The promoter itself repeats verification before applying any tag. |

A failed later build can leave already verified, signed images addressed only by
digest. No tags are promoted until the entire expected set succeeds. Platform,
index and complete-set manifests are distinct scopes; evidence from one scope
cannot authorize another. Registry copies and SBOM/referrer attachments occur
only in the publication workflow. PR jobs preserve temporary workflow artifacts
without publishing releases, containers or OCI evidence.

The CLI release uploads `official-release-manifest.json` and
`official-release-manifest.sigstore.json` with its application assets and SBOMs.
Each published platform/index gets OCI referrers for its manifest and bundle;
complete-set evidence is also attached to each index before promotion. GitHub's
provenance Action records the same named application digests in the repository's
attestation API, which the existing W6 updater consumes. The signed
`releases.json` subject binds the credential page's fingerprint and scripts to
the source commit. Image publication binds the final extracted credential bytes
through the actual asset and image SBOMs, after the mandatory image parity check.

## Existing W6 trust policy

The build adapter includes the existing CLI attestation, Rekor, TUF and TLS source
without modifications. It authenticates live Sigstore trust through the same
embedded TUF root, persists private rollback-protected TUF state, and runs the
same certificate chain, SCT, signed Rekor event time/proof, GitHub issuer, exact
workflow SAN, statement digest and DSSE signature checks. Neither a supplied
trusted-root JSON nor an unsigned receipt can replace that verifier. The caller
also checks exact subject names, source commit, producer scope and provenance.

Production CLI and versioned image identities remain respectively:

```text
https://github.com/ChronoAIProject/NyxID/.github/workflows/release.yml@refs/tags/v<VERSION>
https://github.com/ChronoAIProject/NyxID/.github/workflows/publish-images.yml@refs/tags/v<VERSION>
```

The rolling image publisher additionally verifies its own exact
`publish-images.yml@refs/heads/main` identity. That does not change the updater:
the production machine updater continues to accept only the versioned identity
it already constructs. The issuer is exactly
`https://token.actions.githubusercontent.com`. A different repository, workflow,
issuer or ref does not match. Authentic historical CLI and image bundles run
through the included W6 tests on both native Linux architectures on PRs and
weekly. Unsupported new signing/bundle formats fail closed; changing formats
requires a reviewed producer/consumer compatibility change.

## Administrator provisioning (external prerequisite)

An organization/repository administrator must complete these steps before an
official publication run. This code does not create environments, rules, secrets,
variables, identities, releases or tags.

1. Protect `main`, version tags, and the two publisher workflow files through the
   repository's rulesets and code-owner review. Require `CI Pipeline` for merges;
   restrict version-tag creation and workflow changes to release maintainers.
   Signing authority is the exact reviewed workflow at its branch/tag ref.
2. Create the repository environment **`official-release`**. Configure required
   release/security reviewers, prevent self-review and administrative bypass,
   and allow only `main` and the reviewed `v*` release tags as deployment refs.
   Protect changes to this environment. Jobs that sign or publish application
   artifacts explicitly select this environment; the nonpublishing rehearsal
   does not use it. Do not rely on GitHub's automatic creation of an unprotected
   environment.
3. In that protected environment, set the variable
   **`OFFICIAL_RELEASE_AUTHORIZATION=github-oidc-public-good-v1`** after reviewing
   the authority/ref/custody settings above. This exact policy acknowledgement
   is required by preparation and every publication helper. An absent or different
   value fails publication. This variable is an authorization acknowledgement,
   not a secret or a cryptographic key; its presence cannot prove that the
   environment's administrative protection rules are configured correctly.
4. Permit the pinned GitHub Actions and public-good Sigstore service egress.
   Production signing jobs need `id-token: write` and `attestations: write`;
   release creation needs `contents: write`, and GHCR publication needs
   `packages: write`. The workflows scope those permissions to their jobs.
   Allow the built-in `GITHUB_TOKEN` to write attestations/releases and the
   organization's existing NyxID GHCR packages. Keep the repository public:
   private-instance Sigstore roots are outside W6's current public-good trust.
5. Permit GitHub OIDC issuance in the PR rehearsal job under the organization's
   Actions policy, including the intended PR contributor/fork settings. No
   repository secret or production environment access is required. OIDC or
   transparency-service unavailability fails the rehearsal; it never switches
   to a stored signing key. See [RELEASE_REHEARSAL.md](RELEASE_REHEARSAL.md) for
   its public transparency effects and identity separation.
6. Assign a release/security owner for Sigstore service/root incidents and
   repository/environment policy review. Key custody is ephemeral runner key
   material plus Fulcio's short-lived certificate and Rekor evidence; there is
   no long-lived signing private key to provision or rotate in GitHub secrets.
   Revoke future workflow authority through rules/environment/Actions access
   when compromised; historical accepted signatures remain historically valid
   under W6. Do not rewrite trust or omit Rekor to recover availability.

Until steps 1–4 are complete, official publication is unavailable or lacks the
required administrative custody protection. In particular, step 3 deliberately
fails publication when missing. CI policy tests and PR artifact validation remain
available without this environment or its variable. A real successful rehearsal
is required hosted evidence, but cannot substitute for the production custody
review. Existing APK durable archive provisioning and Renovate/private-CMA
requirements are separate prerequisites documented in [BUILD_SECURITY.md](BUILD_SECURITY.md).

## Release readiness checklist

Use this checklist with the provisioning instructions above and
[BUILD_SECURITY.md](BUILD_SECURITY.md). Each item states the effect of missing
setup and the completion evidence. No new signing secret or custom trust root is
needed.

- [ ] **Repository custody:** Protect `main`, version tags and publisher workflows;
  require **CI Pipeline** and name a release/security incident owner. Missing:
  review/custody gap. Verify effective rulesets, code-owner coverage and owner.
- [ ] **Production environment:** Protect `official-release` with reviewers,
  self-review/bypass prevention and allowed `main`/`v*` refs; set
  `OFFICIAL_RELEASE_AUTHORIZATION=github-oidc-public-good-v1`. Missing/wrong value:
  publication blocked; the variable alone does not prove custody. Verify
  environment rules and the exact variable in repository settings.
- [ ] **Signing identity/trust:** Permit pinned Actions, GitHub OIDC and public-good
  Sigstore/TUF egress; retain the public repository and exact identities above.
  Missing: signing/verification fails. Verify both native **W6 Signing
  Compatibility** jobs and an authorized publisher's mandatory verification.
- [ ] **Publication permissions:** Allow job-scoped `GITHUB_TOKEN`
  `contents: write` for releases, `packages: write` plus GHCR package access for
  images, and `attestations: write`/`id-token: write` for production signing.
  Missing: affected operation fails. Verify Actions/package access settings and
  authorized publisher results.
- [ ] **PR rehearsal OIDC:** Permit the rehearsal job's `id-token: write` under the
  intended PR/fork policy. Missing: required rehearsal fails. Verify **OIDC
  Rehearsal and Production Rejection** and its retained evidence, without
  production environment access or secrets.
- [ ] **Update bots:** Activate Renovate for NyxID; keep Dependabot version updates
  enabled. Missing: corresponding update proposals are absent. Verify Renovate's
  dashboard/run and Dependabot update-job results; proposed hashes, locks and pin
  receipts still need review.
- [ ] **Private CMA access:** Grant Renovate read access to `ChronoAIProject/cma`.
  Missing: CMA lookups/proposals fail. Verify a successful private CMA revision
  lookup in Renovate's run/dashboard.
- [ ] **Durable APK archives:** After merge, dispatch **Provision APK Archives** on
  **main** with the full reviewed SHA in `catalog_commit`; retain its
  `build-inputs-apk-YYYY-MM-DD` releases. Missing: durable HTTP 404s warn and use verified
  upstream archives, whose removal can break builds. Verify **APK durable
  archives present** passes; missing-archive failures are non-blocking only on PRs.
- [ ] **Scheduled refresh and PR checks:** Enable schedules, Actions PR creation
  and the maintenance publisher's `contents: write`/`pull-requests: write`
  permissions. Missing: no published refresh; inputs eventually go stale.
  Verify **Weekly Build Input Refresh** produces verified archives and a PR;
  start PR CI through a maintainer push or account-based close/reopen, since
  `GITHUB_TOKEN`-created PRs do not start it.
- [ ] **Continuing reviews:** Assign refresh/disposition owners. Missing reviews:
  catalog/CA checks older than 14 days, CA sources older than 90 days and expired
  monthly/90-day/history-only annual dispositions block their gates. Verify freshness, history scan,
  dependency audit and scanner policy jobs; follow [RELEASE_SCANNING.md](RELEASE_SCANNING.md).
- [ ] **Private hosted deployments, when used:** Supply reviewed CMA context and
  use `build-local-image.py`. Missing context/checks: no qualifying build. Verify
  passing identity/scan/SBOM receipts and the deployment's registry digest per
  [MACHINE_HECA_CMA_IMAGE.md](MACHINE_HECA_CMA_IMAGE.md); public desktop CI ends
  before private final layers.
- [ ] **Independent credential-manifest hosting, when configured:** Host verified
  release `releases.json` and set `RELEASE_INTEGRITY_MANIFEST_URL`. Missing:
  independent manifest comparison is unavailable. Verify page retrieval and the
  signed-release fingerprint per [RELEASE_INTEGRITY.md](RELEASE_INTEGRITY.md).
- [ ] **Hosted acceptance:** Allow required native Linux/macOS runners and packaged
  browser prerequisites, including native root/Landlock ABI >= 6. Missing:
  required jobs fail. Verify **CI Pipeline** is green at the reviewed PR head,
  including packaging, scans, SBOM closure, signing rehearsal and machine checks.

## Verification and incident handling

A maintainer can rebuild the adapter with the locked workspace and verify a
saved `verification-request.json` against the saved bundle. The request contains
only the bundle path, expected exact identity, and SHA-256 digests. Generate a new
request from the independently reviewed manifest and expected publication plan;
do not accept an expected identity merely because it appears in downloaded
metadata. `check-official-release.py verify` performs both contract reconstruction
and W6 verification, and is the supported publication gate.

The browser's optional deployment-host `RELEASE_INTEGRITY_MANIFEST_URL` is a
separate runtime configuration. An independently hosted copy of `releases.json`
should be copied only from a verified official release, with its manifest and
bundle retained for administrator verification. No arbitrary CDN publish command
runs in the official workflows. Failure to configure that runtime URL retains the
existing credential-entry behavior described in [RELEASE_INTEGRITY.md](RELEASE_INTEGRITY.md).

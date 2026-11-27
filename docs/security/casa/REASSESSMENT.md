# CASA reassessment handoff

NyxID is preparing its reassessment with TAC Security as part of the Google
OAuth verification process. The assessment and Google verification are tracked
in [issue #2231](https://github.com/ChronoAIProject/NyxID/issues/2231).

## Merged remediation

| Pull request | Scope | Status |
| --- | --- | --- |
| [#3016](https://github.com/ChronoAIProject/NyxID/pull/3016) | Application file-intake inventory | Merged into `dev` |
| [#3017](https://github.com/ChronoAIProject/NyxID/pull/3017) | Authentication hardening | Merged into `dev` |
| [#3046](https://github.com/ChronoAIProject/NyxID/pull/3046) | Follow-up CI corrections | Merged into `dev` |

The [file-intake inventory](../FILE_INTAKE.md) and [security architecture](../../SECURITY.md)
document the relevant application controls. The merged pull requests contain the
implementation history and CI results.

## Reassessment evidence

The security owner maintains the control-by-control evidence worksheet for TAC
and collects deployment-specific evidence for the assessed release. The following
items remain to be confirmed before final attestation.

- [ ] Confirm that production runs the assessed remediation release.
- [ ] Confirm effective security configuration and complete authenticated retesting.
- [ ] Record Google OAuth client registration and supported mobile sign-in results.
- [ ] Attach release-specific security and dependency evidence.
- [ ] Submit the updated responses and attachments to TAC Security.
- [ ] Record TAC's reassessment decision and Google OAuth verification outcome.

The GitHub issue remains open until the external verification and production
acceptance checks are complete.

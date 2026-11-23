# iOS Build 7 and staging QR rollup review — 23 November 2026

Branch: `rollup/ios-build7-qr-recovery`.
Integration base: `origin/dev` at `192187b322c7ad0d944abae25b6e5a2f31de4812`.
The branch initially used `3f50bda36`, then merged the newer dev thread-column changes.

## Source PR assessment

| PR and reviewed head | Verdict | Included once |
| --- | --- | --- |
| [#2759](https://github.com/ChronoAIProject/NyxID/pull/2759), `882cc38f1` | Still relevant. The verified Build 7 mobile behavior is missing from dev. The original branch conflicts with current upload recovery. | `5221a9e2c`, `31f121cd2`, `882cc38f1`, reconciled with the current lifecycle. |
| [#2756](https://github.com/ChronoAIProject/NyxID/pull/2756), `b48e42d4f` | Its implementation duplicates #2759. Its handoff documentation remains useful. | `b48e42d4f` documentation. The restored UI and folded-Duo error-scroll indicator are already in #2759. |
| [#2758](https://github.com/ChronoAIProject/NyxID/pull/2758), `b2a7d20ee` | Its staging QR addition remains relevant. Its clean merge status applies to the Build 7 feature branch, not current dev. | QR implementation `0d9d8f948` and module ordering `93eb64083`, originally from #2753. Omit the RC merge, which would carry the duplicate restoration again. |

The PRs remain open. This rollup does not merge or close them.

Sol supplied a read-only overlap plan and identified the need to disclose the
staging environment before the approver confirms a session.

## Integration decisions

- Restore the grouped New chat / More controls, anchored chat menu with Settings,
  ordinary-phone compact composer, inline sign-in errors, validated image/PDF
  previews, horizontal attachment row and per-file failure presentation.
- Keep current dev's durable drafts, selected draft model, explicit upload resume,
  reselection, blocked discard retry, exact receipt cleanup, removing/busy feedback,
  and owner/composer lifetime fences.
- Prepare local preview metadata independently while keeping byte reads, uploads,
  draft writes and server cleanup serialized.
- Keep source-branch preview/geometry coverage and move the old attachment
  component's useful behavioral coverage into the React Native Jest suite.
  Add explicit tests for resume/reselection, discard retry and per-file removal.
- Preserve the current redacted device-login Debug implementations, job health
  state and application management observations when integrating AppState changes.
- Move approval/denial tracing spans to the shared bound implementations so
  local and staging decisions retain identical row/session trace coverage.
- Register the PDF companion pod in the native synchronization check.
- Show the trusted staging hostname and a staging-specific approval label from
  the server-returned staging binding. Requester-supplied origin/name hints do not
  grant that label. Add both Chinese translations and regenerate locale barrels.

## Compatibility assessment

No known breaking public API or stored-data change was identified in the source
review or completed client validation. Backend compilation and the cross-server
integration test still require CI verification before this becomes merge-ready.

The existing device-login service entry points keep their signatures and delegate
to new optional binding helpers. The internal app-identity approval caller is
updated with the optional staging match. Existing local device and Agent Key
flows remain independent of the camera-only staging route.

Staging QR support is additive and defaults off through
`auth:staging-qr-login`. The production caller must be a first-party human
session with live entitlement; both environments require active personal
accounts with the same verified email. Approval checks MFA and exact request /
account binding. Staging receives a short-lived, purpose-specific signed
assertion at a fixed endpoint; production bearer credentials are not forwarded.
The session is issued and privately collected by staging.

Enabling the feature requires backend support on both environments, the mounted
production public verification key on staging, and an updated iOS binary.
Production signer rotation requires updating staging's pinned public key.
See [the rollout instructions](../DEVICE_LOGIN_PROTOCOL.md#production-ios-approvals-for-staging).

The PDF renderer is a new local native module with bounded cache-file reads and
page-zero rendering. Existing binaries without the module retain the document
fallback through optional native module discovery. A native rebuild is needed
for PDF thumbnails. Native synchronization includes the companion pod.

## Completed validation

Commands use the repository's CI pnpm version, 10.17.1.

- `pnpm test`: **8,438 passed**, zero failures, cancellations or skips.
  Includes locale parity/tooling, upload lifecycle, scanner and account login
  checks, and the native Swift PDF rendering proof.
- `pnpm test:jest --runInBand`: **163 passed across 14 suites**.
- `pnpm typecheck`: passed.
- `pnpm check:native-sync`: passed; 34 direct native dependencies,
  five companion pods and four permissions.
- `pnpm sync:i18n`: passed; locale checks also passed in the full Node suite.
- Focused chat, upload, header, crew and auth suite: **415 passed**.
- Final login-screen and request-routing suite: **200 passed**.
- Frontend `vitest run src/hooks/use-feature-flag.test.tsx`: **6 passed**.
- `git diff origin/dev --check`: passed.

Rust / Cargo are not installed on this Mac (PATH and usual install locations
checked). Backend compilation, Rust formatting / clippy and
`staging_qr_two_servers_approve_only_the_reviewed_staging_account` must run in
CI with the configured MongoDB replica set. The carried integration test checks
entitlement gating/revocation, verified-account matching, request binding,
separate signing keys, staging delivery, ordinary local approval, denial,
repeat approval and production-bearer rejection.

Signed app builds, physical-device checks, staging deployment and feature
activation are outside this source integration. In particular, check ordinary
iPhone and folded Duo layouts at large text sizes and complete a real staged
approval before enabling the feature for users.

## Other ongoing work

The separate Heca `ios-composer-notification-fixes` worktree has concurrent local
changes. This rollup takes its implementation from the reviewed PR commits and
does not include that worktree's newer notification or composer edits. Reconcile
that branch against this rollup when preparing its eventual PR to avoid
reintroducing the same attachment restoration.

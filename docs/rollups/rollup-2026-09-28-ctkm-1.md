# Rollup: 2026-09-28 ctkm-1

Based on `main` at `567dd3edae1309f6db4cb313928d408752562d3a`.
This rollup repairs X webhook admission and the configuration guidance used to
diagnose missing channel activity.

## X webhook repair

X documents `X-Twitter-Webhooks-Signature-OAuth2`, authenticated with the app's
OAuth 2.0 Client Secret. NyxID previously verified only the legacy signature
header with the OAuth 1.0 consumer secret. A modern-only delivery could therefore
receive the immediate HTTP acknowledgment without being admitted or counted.

- Verify the modern header when present; retain legacy verification when absent.
  Reject an invalid modern signature without downgrading to legacy verification.
- Reject JSON object CRC tokens to prevent challenge responses from being reused
  as authenticated webhook bodies.
- Prefer the OAuth Client Secret for CRC, with a legacy consumer-secret fallback.
  Allow app bearer plus either signing secret for webhook readiness.
- Display the shared `/api/v1/webhooks/channel/x/platform` callback on X bots,
  matching the URL registered with X.
- Correct credential guidance and describe encrypted Chat as an explicitly
  selected metadata notification.
- Log fixed rejection stages and successful admission identifiers without
  secrets, plaintext or encrypted payloads.
- Preserve all unchanged channels when the setup lease is busy, and registered
  active channels after read-only provider failures once credentials and billing
  admission have passed. Missing credentials, failed billing admission and
  uncertain subscription mutations still stop delivery. Persist incomplete event-selection changes so cleanup also
  covers interrupted removal of Chat or other events. The update handler
  reconciles immediately; delivery pauses until reconciliation succeeds.
- Persist cleanup state before first-time subscription effects, including
  unmetered DM setup. Only completed read-only setup failures retain polling
  fallback. Preserve existing active channels during temporary OAuth refresh
  failures while rejecting revoked credentials.
- Finalize incomplete setup markers with a locally authored failure cause and
  one failure audit, including first-time setup and event-selection changes.

The setup progress tracking and its regression cases originate in the X portion
of [PR #1644](https://github.com/ChronoAIProject/NyxID/pull/1644), adapted to the
current encrypted Chat and own-post selections. Its unrelated ownership-transfer
changes are outside this rollup.

## Validation and incident closure

Regression coverage exercises both signature formats, exact-body authentication,
CRC signing-oracle rejection, no downgrade, encrypted Chat admission through the real HTTP router and
owner API, failed callbacks retaining activity, deduplication, metered admission,
legacy callbacks and setup recovery. Browser coverage checks managed X setup and
activity display with zero ordinary messages. Completed check results belong to
the source PR and its CI run.

Production closure requires a fresh X event after deployment, an owner-visible
activity ID, and receiver evidence for a subsequent opted-in typed callback.
HTTP 200 from NyxID and HTTP 202 from an agent are acknowledgments, not proof of
processing. Encrypted Chat does not expose plaintext or grant reply authority.
See [Channel activity](../CHANNEL_ACTIVITY.md) for the receiver declaration and
owner-consent contract.

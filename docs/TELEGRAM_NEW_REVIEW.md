# Telegram New implementation review

Date: 2026-09-11. Scope: the `telegram-new` channel option and administrator manager configuration. The earlier channel-onboarding plan reviews do not certify this implementation.

An independent Opus agent reviewed the implementation and then checked the bounded fixes. Its final assessment was: **“All listed closures verified against the settled source. No remaining holes in the bounded contract.”** The reviewer read code; it did not execute tests or observe Telegram clients. This assessment is not approval for production enablement.

The review led to these changes:

- Private Telegram identity and one-use consent name the exact bot and stable NyxID destination. Browser approval cannot substitute for Telegram approval.
- Telegram's creator field is not treated as current ownership. A second management event stops delivery; fresh claims of that bot are refused. Verification and setup retries cannot bypass suspension.
- Creation provenance includes Telegram's original message timestamp and a manager observation identifier. Provenance is immutable. The observation start is saved only with successful manager webhook verification and survives same-manager token rotation.
- Unprovisioned creations can be recovered for 60 minutes; the ordinary browser request lasts 15 minutes. First provisioning consumes the creation identity transactionally, so deletion cannot make it available for another claim.
- Before automatic token retrieval, NyxID checks management revision, provenance, freshness, current webhook, pending deliveries, and Telegram-reported errors, then reads the revision again. Bot insertion checks it inside the transaction as well.
- A later received update is not used as proof that all earlier events were delivered. A recorded manager delivery error after creation disqualifies that creation. Pending updates cause a retryable refusal; busy managers can therefore temporarily delay connections.
- Manual Telegram and Telegram New share remote identity and owner-quota checks. Identity-specific configuration locks avoid blocking unrelated manual registrations; child setup and deletion use a distinct lock namespace.
- Duplicate Telegram-account requests and revoked destination access no longer trap the provider in permanent webhook retries. An actor can cancel an unstarted request after losing organization access.
- Pending bot rows and encrypted credentials survive failed webhook setup. Cleanup checks the exact owned webhook, including when local setup is pending, and preserves a different application's callback.

The reviewer withdrew three proposed shortcuts after checking the counterexamples: filtering management changes only by creator identity; treating a later received update as proof of complete event delivery; and dropping initial pending Telegram updates when timestamp/observation checks already exclude old provenance. It also withdrew a deduplication-window concern: an eligible bot at revision 1 cannot have evicted any of its 64 retained management update IDs.

Remaining live checks and operator steps are in [TELEGRAM_NEW.md](TELEGRAM_NEW.md). In particular, validate native Telegram creation, event shape/timing, token acquisition, return behavior, and ownership changes with a dedicated staging manager before customer enablement.

Initial local validation completed after the review: 17 focused Telegram New backend tests, 399 channel backend regressions, 34 channel CLI tests, and 67 frontend tests passed. The production frontend build, ESLint (zero errors; 27 existing warnings), Rust formatting, and diff whitespace checks passed. MongoDB transactions were exercised on a real local replica set; all Telegram HTTP calls in these tests were simulated. No live provider or deployment gate was closed by these results.

## Integration review — 2026-09-14

After rebasing onto `main` at `04c44f5b`, the same independent Opus reviewer checked the conflict resolutions and updated interfaces. Verdict: **“clean — no newly-introduced regressions found.”** This was a read-only code review; it did not repeat the earlier protocol review or execute tests.

The reviewer confirmed that existing Telegram still exposes its token form, the new adapter remains separate, and non-Telegram registration and deletion retain their previous behavior. The review also covered X's shared OAuth credentials, polling and reconnect, WhatsApp managed configuration, frontend flow dispatch, return links, and CLI handoff. It verified that the small Oracle enrollment Clippy simplification preserves behavior. Assembled-router construction is exercised by the channel-handler regression suite.

Current local test results and the remaining live Telegram requirements are recorded in [Telegram New](TELEGRAM_NEW.md#local-verification-record).

# Telegram forwarding and demo follow-up

The product owner accepted the current gateway content gap for the demo. No CMA
change or additional message-body persistence is part of this PR.

NyxID provisions Telegram NyxBot gateway channels with `nyxid_relay` sources. On
these channels, NyxBot receives the gateway's plain text plus `telegram_input::relay_input`
metadata: authenticated original-message eligibility and own/quoted file handles.
Forward-origin names, quote/external-reply text and hidden link URLs are **not
available** through the current gateway. Own-file indexes remain an exact prefix
of the stored attachment list; quoted files do not change the gateway payload.
Forwarded, quoted, edited or unverified input cannot authorize pairing, controls
or action-card decisions. Missing provenance fails closed.

A separate CMA change can close this gap by forwarding selected Telegram text,
entities, captions, quote/external-reply content and authenticated provenance in
a versioned envelope bound into the activity digest/seal for both `nyxid_relay`
and native Telegram sources. Native-source events without this evidence remain
plain text and cannot decide cards, pair or run controls; NyxID never creates
native-source NyxBot channels. CMA could also expose and correlate external-reply
files for its own providers/rendering, preserving media admission and file indexes.

An earlier proposal exists at
`~/.heca/plans/nyxbot-botfather-ux/cma-forwarding-companion.patch`. It is
non-normative, unapplied and unverified against a compiled CMA gateway. Future
CMA work requires its own repository review and tests.

The following review items are **deferred by owner for demo**:

- F2: Separate sender-owned control text from rich reference input before allowing quoted owner controls (`backend/src/services/telegram_input.rs`, `backend/src/handlers/nyxbot.rs`, `backend/src/handlers/nyxbot_thread_follow.rs`, `backend/src/services/platform_manager_transport.rs`); the current conservative quote refusal stays in place.
- F3: Hoist UTF-16 encoding out of the entity loop (`backend/src/services/telegram_input.rs::linked_text`).
- F5: Restrict proof scrubbing to generated alphabets/case and recognized contexts to avoid ordinary-reference false positives (`backend/src/services/telegram_input.rs::scrub`).
- F7: Bound terminal multipart receipt retention while preserving pending/uncertain receipts and preventing replay after expiry (`backend/src/services/channel_text_delivery.rs`, `backend/src/models/channel_text_delivery.rs`).
- F8: Seed the default help library only at startup and make missing-library reads fail explicitly (`backend/src/services/platform_help_service.rs`).

# Optional CMA Telegram forwarding follow-up

NyxID provisions Telegram NyxBot gateway channels with `nyxid_relay` sources.
Its relay envelope retains the sending message's own text, type and top-level
attachment list. NyxID stores original-message eligibility and the complete
own/quoted file-handle list as metadata, then recovers it for NyxBot through the
exact authenticated message, bot, route key and sender. Own-file download indexes
remain unchanged. Recovery of this metadata works with the current CMA relay contract and
requires no companion gateway change.

Two optional CMA improvements remain separate from this NyxID PR:

1. Native Telegram-source channels could forward selected text/entities,
   captions, quote/external-reply content and an original/forward provenance
   fact derived at authenticated ingress and bound into the activity seal.
   Without that evidence NyxBot accepts plain text and fails closed for card,
   pairing and control decisions. NyxID never creates native Telegram-source
   NyxBot channels, so this compatibility behavior is intentional.
2. CMA could extract and correlate `external_reply` files for its own providers
   and rendering. Today its relay verifier accounts only for the sending
   message's top-level media. NyxID therefore keeps quoted files out of the
   gateway attachment envelope while retaining them for NyxBot itself. A future
   gateway extension must preserve exact file/index correlation and its media
   admission policy.

An earlier proposal is available locally at
`~/.heca/plans/nyxbot-botfather-ux/cma-forwarding-companion.patch`. It is
non-normative, unapplied and unverified against a compiled CMA gateway. Any future
CMA work needs its own repository review and tests; that patch is not included in this local change.

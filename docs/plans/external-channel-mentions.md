# Exact mentions from external channels

## Behavior

An authenticated Telegram or Lark/Feishu group message that exactly addresses a connected native bot reaches that bot's live linked NyxBot or specialist, including a per-chat agent override. Admitted participants do not need personal OAuth merely to address it. Delivered bot-authored mentions execute as bounded guest turns.

The native receiving installation is the target authority. A native chat has no automatic mapping to a web `AssistantGroup` roster. Display names, arbitrary specialist names and another owner's bot cannot select an agent or grant execution permissions.

## Implementation

1. Share bounded native mention parsing within each repository. Telegram pairs text/caption with the corresponding entities, validates UTF-16 scalar boundaries, and matches exact usernames, native user IDs and directed commands. Lark/Feishu compares verified receiving `open_id` values. Broadcasts, conflicting metadata and unrelated display labels do not establish a match.
2. Preserve verified native actor kind through relay, gateway, execution and delayed text/image delivery. Bots stay guests even if their ID overlaps an owner ID; they cannot link owners, approve action cards, queue owner work, create chat settings or open/renew thread following.
3. Retain existing group consent. A group/topic must already permit members explicitly, or through the default after a verified owner has spoken there. Owner-only and private restrictions remain authoritative. A bot needs a positive exact mention even when the group answers all human messages.
4. Atomically spend the existing per-chat `max_group_handoffs` allowance and owner `max_group_handoffs_per_hour` window in MongoDB. Only admitted authenticated human input resets the chat allowance. Native redelivery cannot repeat execution or reset it twice. Busy or uncertain bot attempts remain consumed and silent. Legitimate human Telegram edits must retain their existing direct-relay behavior.
5. Add a generic CMA source opt-in, `admission.bot_messages: native_mentions`, with default denial for existing consumers. Require pinned receiver identity and raw verification; preserve `actor.kind: bot` in the provider Activity. Update public SDK schemas, OpenAPI and operations documentation.
6. Reconcile existing NyxID gateway connections automatically after capability discovery. Trust receiving identity and bot admission only after the gateway confirms the stored policy. Version conflicts and uncertain updates must recover through authenticated management reads without manual reconnect.

## Rollout and compatibility

Deploy CMA readers and all NyxID consumers before enabling `CMAEG_NATIVE_BOT_MENTIONS_WRITES_ENABLED`. The flag defaults to false and gates authoring of the new stored source shape. The authenticated `native_bot_mentions_v1` capability then permits NyxID's management sweep to opt existing connections in. Old gateways retain the compatible human-only policy shape.

Before rolling back to a reader that predates this feature, remove opt-ins through a current reader and expire gateway caches. Disabling the write flag alone does not remove existing definitions. Basic mentions do not depend on a separate gateway thread-follow release.

Provider delivery settings still determine which events reach NyxID/CMA. Local acceptance tests establish processing of authenticated delivered events; they do not establish a live provider's configuration or delivery.

## Verification

Regression coverage exercises signed native ingress, CMA Activity normalization and NyxID guest specialist execution, with no participant OAuth/binding records. Negative cases cover foreign ownership, self/unknown actors, wrong targets and namespaces, broadcasts, malformed UTF-16, owner-only groups and owner-ID spoofing. Additional cases cover concurrent budget spending, busy/retry behavior, redelivery, human edits, active followed children and delayed text/image delivery.

Validation uses an explicit writable MongoDB replica set so database tests cannot silently skip. Required checks include the focused feature suite, full NyxBot suite, relevant channel/thread/guest tests, Rust formatting/checking/lint, CMA gateway/core and SDK tests, and generated contract/configuration checks. The final PR records actual results and CI evidence.

Implementation uses the requested Heca session with `gpt-6.1-sol`, `xhigh` and full access. The coordinating agent personally reviews the complete changes and returns findings to that same implementation session. No review subagents are used.

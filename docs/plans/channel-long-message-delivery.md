# Long IM replies: investigation and delivery contract

The reported Lark screenshot shows a URL continuing in the next message. That
demonstrates an unsafe split, but does not itself prove that text was lost. The
code contains both unnecessary splitting and separate lossy truncation paths.
This investigation inspected local source; it did not identify the deployed
transport or inspect the contents of the reported production conversation.

## Findings

| Layer | Existing behavior | Consequence |
| --- | --- | --- |
| NyxID `services/channel_thread_service/delivery.rs::components` | Every text and caption was cut at 2,000 UTF-8 bytes, regardless of platform. | Chinese reaches the cutoff earlier; URLs and words can be cut in half. The full string is retained across components, subject to successful delivery. |
| CMA `cmaeg/src/delivery/render/relay.rs` and `tools/context.rs` | Generic relay capabilities advertise 4,000 units; the renderer also clamps to 4,000. `render/text.rs` counts UTF-16 units after escaping, prefers paragraphs, and otherwise cuts at the limit. | Lark/Feishu messages can be divided before reaching NyxID even when one native message would fit. A URL can straddle the boundary. |
| NyxID `handlers/nyxbot.rs::bounded_reply` | Calls `excerpt` with `MAX_CHANNEL_REPLY_CHARS = 8000`. | The remainder of the answer never reaches the outbound transport. |
| NyxID `handlers/nyxbot_late_delivery.rs::answer` and `nyxbot_images.rs::append_failure` | Reserve space for notices within the same 8,000-character budget. | Failed turns and image notices can also remove answer text. |
| NyxID `handlers/nyxbot_chats.rs::automation_reply_with_note` | Fits a single platform message, replacing overflow with a shortened-reply notice and a link to the app. | The IM receives an excerpt, even though the full result exists in the assistant transcript. |
| CMA `cmaeg-core/src/message.rs` and `cmaeg/src/turns/projection.rs` | Reply API text fields are bounded at 32 KiB; streamed projection has a separate 256 KiB text budget. | Deleting NyxID's excerpt calls without coordinating transport limits can turn truncation into refused delivery. NyxAgent can produce up to 2 MiB. |

CMA source inspected: commit `9c9f089` (2026-10-13), local
`wa-channelbots-cma` checkout. It is evidence of that implementation, not proof
of the deployed gateway version.

## Chunking ownership at the CMA/NyxID boundary

For NyxID-relayed replies, the intended flow is one complete logical reply from
CMA to NyxID, followed by platform-specific chunking inside NyxID immediately
before provider sends. CMA must stop applying its native message-size splitter
to that relay path. Its direct Telegram delivery still needs native chunking;
interactive controls and edits retain their single-message constraints.

CMA's `delivery/send.rs::reply_inner` currently loops over rendered parts and
posts each independently through `delivery/relay_client.rs`. The request carries
the original inbound `message_id` and this part's text, with no logical-response
ID, sequence, part count or completion marker. NyxID's `AsyncReplyRequest` has
the same ungrouped contract. NyxID cannot reliably reassemble those calls by
waiting or grouping on the inbound ID: intentional separate replies share that
ID too. A single complete request avoids this ambiguity and needs no durable
message-body queue. If transport chunking becomes necessary, it needs an
explicit bounded framing/completion contract before platform delivery.

There is also a conditional delivery gap in CMA's current loop: the first part
can consume the single-use reply token, while subsequent parts require a
channel key. If that key is absent, CMA stops and reports partial delivery.
This is a source-level failure path, not a confirmed cause of the reported
production incident. One authorized logical request lets NyxID own all its
provider sends while preserving live authority checks and partial-send receipts.

## Implementation follow-up

The initial local patch covered native thread splitting only. The coordinated
NyxID implementation now extends that policy to ordinary text replies, records
logical reply outcomes, enforces the 2 MiB wire contract, and removes NyxBot's
8,000-character answer excerpts. The current contract and rollout requirements
are documented in [Complete IM replies](../CHANNEL_COMPLETE_REPLIES.md).

The Event Gateway handoff is tracked in
[CMA #1029](https://github.com/ChronoAIProject/cma/issues/1029). CMAEG's default-off
complete-replies flag must remain off until the NyxID rollout and coordinated
latency checks pass. Proactive automation sends, interactive/media layouts and
non-IM transports retain their separate contracts.

## Investigation validation

The original thread planner produced four messages for a Lark reply that fit in
one native message. The initial isolated harness reproduced that failure and
passed after the first fix. A later regression reproduced a cut between a
backslash and its escaped character; the updated splitter keeps that pair
intact. Full HTTP/database and gateway-thread regression tests accompany the
coordinated implementation; see the contract document for the command.

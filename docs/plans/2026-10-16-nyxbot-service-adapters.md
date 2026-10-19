# NyxBot service adapters with declared capabilities

Proposal, 16 October 2026. This document describes an implementation direction;
it does not enable providers or change existing channel or voice behavior.

NyxBot should accept new messaging and voice integrations through registered
adapters. Each adapter describes the operations its implementation supports,
and a resolver reports which operations the selected connection can currently
use. NyxBot retains agent identity, instructions, conversation state, tools,
permissions and memory. Adapters translate provider protocols and effects.

The initial use case is a phone or SMS conversation in which NyxBot searches an
approved Notion FAQ during a turn and returns the answer through Telnyx, Twilio
or ElevenLabs. The same agent can serve several channels without copying its
instructions and tool grants into each provider.

## Existing foundations

- `services/channel_platform.rs` already defines `PlatformAdapter`, outbound,
  media and thread capabilities, inbound messages and delivery outcomes.
- `services/channel_adapters/mod.rs` is the channel registry;
  `channel_platform_catalog_service.rs` derives the public inventory from those
  adapters. `GET /api/v1/channel-platforms` already exposes this metadata.
- `services/voice/` has OpenAI and xAI runtimes, credential admission, session
  ownership, transcript handling, confirmation and billing coordination.
  `docs/chat/11-voice.md` describes normalized provider events and commands.
- Catalog services, `UserService` connections and platform-key bindings already
  own credential resolution and access. The catalog also has general proxy and
  inference metadata. Those fields do not prove a live-call adapter exists.
- Current assistant voice routes require a first-party human and a private
  owner thread. Phone callers must use a separate channel admission path; this
  proposal does not relax that restriction.

## Boundaries

Use one discovery vocabulary and a registry of typed interfaces. Avoid one large
trait whose methods mostly return unsupported errors.

| Interface | Responsibility | Initial examples |
| --- | --- | --- |
| Messaging | Receive messages, send replies, report delivery and attachment limits | Existing channel adapters; Telnyx SMS; Twilio SMS |
| Call control | Admit incoming calls, dial, answer, hang up, transfer where implemented | Telnyx; Twilio |
| Realtime conversation | Receive final utterances, stream responses, handle interruptions and session closure | Telnyx Conversation Relay; Twilio ConversationRelay; ElevenLabs Speech Engine |
| Delegated conversation | A provider-hosted conversational agent asks NyxBot to perform work | Optional later Telnyx/ElevenLabs integrations |

One implementation may expose multiple interfaces. A vendor can have several
adapter profiles: `telnyx.messaging`, `telnyx.conversation_relay`,
`twilio.messaging`, `twilio.conversation_relay`, `elevenlabs.speech_engine`.
These are proposed identifiers, not currently registered adapters.

Retain ordinary SaaS tools, including Notion, in the existing catalog/MCP path.
They do not need a new channel adapter. Existing inference integration likewise
remains the model execution path. The registry can describe how these services
participate without replacing their working protocols.

Distinguish the phone carrier, speech provider and agent runtime. Twilio
ConversationRelay using an ElevenLabs voice is one Twilio integration; it does
not necessarily require a separate user-owned ElevenLabs connection. Telnyx SIP
feeding an ElevenLabs agent is a different, explicit composition. Only publish
compositions whose routing, identity and credentials are implemented and tested.

## Three records with different purposes

**Adapter descriptor:** a versioned declaration shipped with the implementation.
It identifies the protocol, interfaces, supported operations, limits, setup
fields, credential requirements and documentation. Provider marketing features
are not capabilities until our adapter implements them. An administrator cannot
enable executable behavior by editing JSON flags. Discovery comes from the
registered code; a generated manifest may be retained for review.

**Binding:** persisted owner configuration referencing the adapter, existing
catalog service/connection, agent, permitted provider resources such as a phone
number, selected voice/model and channel policy. Reuse channel records where
their semantics fit and reference the existing credential store. Do not duplicate
secrets, prompts, Notion content or grants in an adapter record. Bindings carry a
revision so active sessions cannot silently change account or agent.

**Resolved availability:** computed for the caller and selected binding. It
combines implemented capabilities, deployment flags, compatible catalog settings,
current credential/grant access, provider resources, account features and billing
readiness. A supported operation can be unavailable with a specific reason, such
as `connection_required`, `number_not_configured`, `scope_missing`,
`billing_unavailable`, `verification_required` or `disabled`. Unknown features
remain unavailable. This response is informational; execution rechecks authority.

Represent credential selection as alternatives: a user connection reference or
an authorized catalog platform-key reference. Resolve OAuth refresh and platform
ACLs through existing services. Never silently switch between a user's account
and a shared platform account.

## Capability contract

Use typed, versioned groups rather than one flat list of vendor booleans.

| Group | Information to represent |
| --- | --- |
| Messaging | Inbound/outbound text and media, reply/edit support, size/part limits, acceptance and delivery evidence |
| Calls | Inbound/outbound support, answer/hangup/transfer/DTMF operations and prerequisites |
| Conversation | Final and interim transcripts, streamed output, caller interruption, output cancellation and playback evidence |
| Instructions | External NyxBot execution, startup prompt override, provider-side context append, native prompt replacement and its timing |
| Lifecycle | Reconnect behavior, event replay/deduplication evidence, session limits, retry guarantees |
| Usage | Reported or measured units, finalization evidence, supported billing path |
| Setup | Credential modes, required number/application/webhook resources, supported composition profiles |

For timing-sensitive features use enums, for example
`unsupported | session_start | between_turns | native_live`, rather than
`dynamic_prompt: true`. Provider-side instructions and NyxBot-side instructions
are separate fields: a text relay does not own a provider prompt to update.
Keep descriptive limits, implementation support and live readiness distinct.

An illustrative resolved operation result is:

```json
{
  "adapter_id": "twilio.conversation_relay",
  "contract_version": 1,
  "binding_revision": 4,
  "operation": "receive_call",
  "implemented": true,
  "availability": "unavailable",
  "reason": "number_not_configured"
}
```

This illustrates the response shape only. No Twilio channel implementation is
being claimed. Capability declarations must be covered by contract tests against
the associated operation handlers. Older clients ignore unknown additive fields;
servers reject unknown operations and unsupported adapter versions.

## Runtime ownership

The shared session coordinator resolves an approved binding and caller access,
then selects the appropriate NyxBot conversation. Each call has a stable session
identity and its own conversation unless an authorized continuation policy says
otherwise. Public callers do not inherit the owner's private history or tools.
FAQ access uses existing guest/specialist grants; caller ID alone does not confer
owner authority. New channel capability metadata never grants access.

Provider adapters verify inbound requests, correlate them with the configured
account/resource, and emit typed events such as `SessionStarted`, `UserUtterance`,
`PlaybackProgress`, `Interrupted`, `UsageObserved` and `SessionEnded`. Events carry
provider identity, local session/generation and utterance/reply correlation.
Keep distinct variants for different evidence; receipt of text by a provider
does not prove that a caller heard it. Do not emulate every provider as OpenAI.

The coordinator admits final utterances to normal NyxBot turns. Interim speech
may update presentation but cannot execute tools. It sends response chunks,
progress notices, supported playback cancellation and close commands through
the relevant interface. Messaging retains its asynchronous reply contract and
must not be routed through a permanently open voice session.

On interruption, stop playback where supported, fence stale output and reconcile
what was heard. Cancellation does not undo an already dispatched tool effect.
An interrupted call or socket loss must not automatically replay a dial, SMS or
agent turn whose outcome is uncertain. Retain the existing durable turn,
approval and operation identities; bound queues and preserve ordering.

The browser voice path stays first-party human-only. Extract reusable internal
coordination after validating both authority models; never feed phone webhooks
directly into its human HTTP endpoints or weaken `private_thread` admission.

## Dynamic instructions and the Notion FAQ

For external NyxBot execution, resolve the agent's instructions at turn admission
and record a revision. Explicitly supported changes take effect at the next turn
boundary; a running turn retains its snapshot. A mid-call change to identity,
grants or credential binding requires live authority revalidation, not only a
prompt reload.

Notion FAQ retrieval is ordinary NyxBot tool execution, independent of the voice
adapter. The agent searches/reads the authorized pages during the call and uses
the results in its response. New reads can observe edits; an existing spoken
answer is not retroactively changed and no automatic Notion push is implied.
Fetched content is reference data, not a source of tool permissions or trusted
role instructions. Prompt and tool policy remain server-owned.

Provider-hosted agents may instead load a prompt snapshot at startup or support
limited context updates. Their descriptors must report the actual timing and
delegation semantics. Attaching Notion directly to such an agent does not make
it the persisted NyxBot. Supporting that mode is an explicit later integration.

## Persistence, delivery and billing

Preserve the channel relay's metadata-only storage contract. Authorized NyxBot
conversation messages use its existing transcript retention rules. Session and
adapter diagnostics hold identifiers, revisions and bounded reason codes, not
raw audio, credentials, prompts or webhook bodies. Provider audio recording
would be a separate explicit feature.

Record selected adapter/contract version, binding revision and provider session
identity on durable call state. Fence workers and revalidate access before new
effects. Revocation stops new work; already-incurred usage still settles.

Report usage evidence through the existing billing system. A descriptor does not
define prices or introduce a new wallet. Account for carrier, speech and NyxBot
model usage according to the actual composition without counting the same charge
twice. The existing Telnyx managed billing pilot covers a fixed nonstreaming text
model only; it does not authorize shared-account call/SMS cost settlement.
Platform call/SMS admission needs an explicit supported billing contract before
paid rollout. BYOK and platform readiness are resolved separately.

## Implementation sequence

1. Introduce the descriptor and resolved-availability contract. Adapt the current
   channel registry behind it and keep the existing channel-platform API shape
   compatible. Reuse existing voice metadata where it represents the same fact.
2. Implement one complete external NyxBot voice profile, initially Telnyx
   Conversation Relay, with explicit number binding, caller admission, a durable
   session and live Notion FAQ turns. Add its SMS adapter through the messaging
   interface. Publish only operations actually implemented.
3. Add Twilio ConversationRelay and Twilio SMS using the same coordinator. Use
   differences found here to refine the shared contract before generalizing more.
4. Add ElevenLabs Speech Engine or Custom LLM as a specific tested profile.
   Keep carrier composition explicit. Consider provider-hosted delegation later.
5. Expose setup/readiness and capability details in the channel UI and agent
   discovery. Backend execution remains authoritative regardless of UI state.

Proposed new module boundaries are `services/adapters/{registry,descriptor,
availability}` and `services/call_channels/{admission,session,coordinator}`.
Existing `channel_adapters`, catalog credential resolvers, NyxBot execution and
billing remain the implementation owners of their domains. Names are provisional;
do not create a second registry of the same channel implementations.

Acceptance requires a real session path with controlled provider fixtures: a
question causes an authorized Notion read, the reply streams, a later read sees
changed FAQ data, and interruptions cannot release stale output or repeat side
effects. Verify webhook duplicates, cross-account resource rejection, guest
isolation, credential revocation, unsupported operations, unavailable bindings,
session closure, uncertain delivery and usage settlement. Confirm the same core
behavior through Telnyx and Twilio before claiming the interface is portable.

## Provider references checked during this investigation

- [Telnyx Conversation Relay](https://developers.telnyx.com/docs/voice/programmable-voice/conversation-relay)
- [Telnyx dynamic variables](https://developers.telnyx.com/docs/inference/ai-assistants/dynamic-variables)
- [Twilio ConversationRelay](https://www.twilio.com/docs/voice/conversationrelay)
- [Twilio voice configuration](https://www.twilio.com/docs/voice/conversationrelay/voice-configuration)
- [ElevenLabs Speech Engine integration](https://elevenlabs.io/docs/eleven-agents/phone-numbers/twilio-integration/custom-llm-integration)
- [ElevenLabs Custom LLM](https://elevenlabs.io/docs/eleven-agents/customization/llm/custom-llm)
- [Telnyx SIP into ElevenLabs](https://elevenlabs.io/docs/eleven-agents/phone-numbers/telephony/telnyx)

These documents establish upstream interfaces. They do not establish live
account eligibility, latency, billing correctness or completed NyxID adapters.

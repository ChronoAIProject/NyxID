# NyxBot service adapters

NyxBot service adapters connect phone and speech services to an owner-configured
NyxBot specialist. The carrier routes the call or SMS, the speech service handles
audio, and NyxBot supplies the instructions, conversation and authorized tools.
An adapter's compiled capability descriptor describes its implemented protocol;
each saved connection also reports its current readiness.

The implementation is behind the default-off `nyxbot:service-adapters` feature
flag. Upgrade **every API replica before enabling this flag**. Older replicas do
not enforce the new adapter-origin fence on tool callbacks. Keep the flag off
during a mixed-version deployment. Existing channel bots and browser voice have
their existing configuration and authority rules.

## Implemented profiles

| Adapter ID | Connection | Credential modes | Composition |
| --- | --- | --- | --- |
| `telnyx.conversation_relay` | Inbound calls, final transcripts, streamed text replies, interruption | User-owned connection | Telnyx carrier and Conversation Relay, NyxBot specialist |
| `telnyx.messaging` | Inbound SMS and replies, delivery receipts | User-owned or allocated platform account | Telnyx messaging, NyxBot specialist |
| `twilio.conversation_relay` | Inbound calls, final transcripts, streamed text replies, interruption | User-owned or allocated platform account | Twilio ConversationRelay with Google `en-US-Standard-A`, NyxBot specialist |
| `twilio.messaging` | Inbound SMS and replies, delivery receipts | User-owned or allocated platform account | Twilio messaging, NyxBot specialist |
| `elevenlabs.custom_llm` | Streaming Chat Completions endpoint for an ElevenLabs agent | User-owned connection | ElevenLabs speech, NyxBot specialist; any carrier is configured separately |

Shared-platform Telnyx voice is unavailable: its inbound Answer and Conversation
Relay APIs do not provide the provider-enforced call duration bound required by
this platform billing implementation. Platform SMS remains available. ElevenLabs
speech and carrier charges belong to the user's external provider configuration.

These profiles do not implement outbound dialing, transfers, DTMF actions, media
messages, message editing, arbitrary speech-provider switching, or automatic
effect replay. A Twilio call using an ElevenLabs voice would be a separate
composition; it is not enabled by selecting `elevenlabs.custom_llm`.

## Connect an FAQ specialist

1. Create a personal specialist in NyxBot's Agents view. Give it the role and
   instructions callers should receive. Connect Notion through the existing
   Services flow, share the intended FAQ pages with that Notion integration,
   grant the connection to the specialist, and set its **Guest access to Read**
   for FAQ lookups. Use a Notion integration whose page access matches the FAQ
   audience. The main NyxBot orchestrator's guest turns cannot use tools.
2. Connect the chosen carrier or ElevenLabs service under Services. Adapters
   reference existing credential storage. Telnyx uses a Bearer credential,
   Twilio uses HTTP Basic account SID/auth token credentials, and ElevenLabs uses
   its API key header. Compatible provider OAuth credentials use the existing
   resolver and refresh flow; this feature does not register a new Telnyx OAuth
   application or create an OAuth consent flow.
3. In NyxBot Settings, open **Phone and speech connections**, then **Add phone or
   speech connection**. Select the profile, specialist and service connection.
   For a carrier, enter the provider account ID, phone-number resource ID, E.164
   phone number and application or messaging-profile ID. A user-owned Telnyx
   connection also requires its account's Ed25519 public verification key.
4. Choose **Verify and connect**. NyxID reads provider resource configuration and
   verifies the number/application association. Saving performs no outbound
   call or SMS. Provider resources must belong to one allocated NyxID owner;
   aliases of the same number/application cannot bypass that ownership.
5. For carrier profiles, configure the application’s inbound URL and terminal or
   status callback URL to the saved webhook URL, using POST. Keep the assigned
   number and application consistent with the verified binding. SMS replies
   receive their own correlated receipt callback URL automatically.
6. Review **Capabilities and readiness**. An implemented operation can still be
   unavailable because of credentials, grants, provider resource configuration,
   catalog operation policy, billing, or the rollout flag.

Compatible catalog base URLs are `https://api.telnyx.com/v2`,
`https://api.twilio.com` and `https://api.elevenlabs.io/v1` respectively. Twilio's
Basic credential is stored as `ACCOUNT_SID:AUTH_TOKEN`; its SID must match the
configured provider account. Node-routed credentials are unavailable for these
server-side adapters. For Twilio voice, leave both the number and application's
voice fallback URLs empty: admission verifies this before its answer/bootstrap
step so an error cannot route an uncapped call into another application.

Use a public HTTPS `BASE_URL` that reaches NyxID's webhook routes, and permit
WebSocket upgrades for Conversation Relay. Preserve the configured public URL
through proxies: Twilio webhook signatures bind the exact callback URL. For a
WebSocket upgrade, NyxID accepts signatures for the configured WS/WSS URL or its
scheme-only HTTP/HTTPS equivalent. The host, port, path, query and ticket must
remain identical; forwarded headers cannot select a different endpoint.
Socket query tickets are authentication material; configure ingress/proxy access
logs to omit query strings. NyxID's HTTP trace spans record paths without queries.

Each call and each inbound SMS receives its own guest conversation. Caller ID
does not grant owner access. The adapter never imports the owner's private chat,
memory or machine permissions. The specialist's ordinary grants and guest policy
remain authoritative for Notion and all other tools.

## Instructions and FAQ changes during a call

The specialist's instructions are loaded for each new turn. An owner can change
the role or instructions during a call; the next admitted turn uses the new
configuration. An already-running turn retains its instruction snapshot. A new
Notion tool read can observe an FAQ edit during the same call. This is ordinary
authorized tool execution, with no copied FAQ index or automatic Notion push.

The descriptor reports `nyxbot_refresh: between_turns` and
`provider_prompt_replacement: unsupported`. NyxBot owns the reasoning prompt;
these transport profiles do not replace a provider-hosted agent's prompt live.
Credential changes, disabled bindings and revoked tool access are checked during
execution, including at tool callback and final dispatch boundaries.

Interrupted speech records only the provider-reported prefix that can be
correlated with a sent reply. A generated reply is not proof that the caller
heard it. Subsequent turns rebuild guest context with this delivery evidence.
Cancellation does not undo a tool operation that already completed, and the
coordinator does not replay such an operation because speech was interrupted.

## ElevenLabs Custom LLM setup

Create an ElevenLabs agent and connect the corresponding ElevenLabs service.
Choose `elevenlabs.custom_llm` in NyxBot Settings and enter the ElevenLabs agent
ID. Select **Show Custom LLM setup** and copy the returned URL, API key and model
into that agent's Custom LLM configuration. Treat the displayed API key as a
secret. It is removed from the dialog when closed or when the NyxID account
changes. Re-verifying the binding changes its revision and invalidates old setup
authentication; copy the newly issued setup afterward.

Select **Create guest session** separately for each conversation. Pass the
returned `extra_body` to the ElevenLabs client using
`ConversationInitiationData(extra_body=...)`. ElevenLabs forwards those values to
NyxID as `elevenlabs_extra_body`. The issued session ID and token correlate one
external conversation with one NyxBot guest conversation. Session issuance and
explicit session closure require the authenticated first-party owner.

Preserve the full, advancing message history in Custom LLM requests and set
`stream: true`. NyxID imports only the final user utterance into its own retained
conversation; provider system, assistant and tool messages grant no authority.
Exact retries, ambiguous history trimming and resets are rejected before another
turn or tool effect. Create a new guest session when starting a new conversation.
An issued session expires after 30 minutes. **End guest session** closes the
session most recently issued in the current settings view. The owner can also
close a known session through
`POST /api/v1/channel-adapter-bindings/{id}/sessions/{sid}/end`.

This profile exposes a Custom LLM endpoint. Phone-number routing into ElevenLabs,
SIP configuration and ElevenLabs speech billing remain in the external provider
configuration.

## Platform accounts and billing

Platform credentials remain encrypted in the catalog service. In **Admin →
Platform Credentials → Platform phone accounts and allocations**, configure the
trusted account and allocate its number/application to a person. Telnyx platform
verification keys come from this administrator-managed trust record. Ordinary
catalog access alone does not establish ownership of a phone resource.

Configure a synchronized credential-class pricing lane in Billing:

- SMS uses **Requests**, one billable request per provider SMS part.
- Calls use **Voice seconds**. Twilio admission reserves 1,800 seconds and
  confirms the provider's `TimeLimit` before starting ConversationRelay.
- NyxBot model/tool usage uses the existing execution billing path independently.
- Extra pricing components, operation-specific lane prices and resale billing
  are unavailable for these carrier profiles; readiness rejects incompatible
  configuration.

A platform allocation requires a tariff version, an HTTPS evidence URL,
confirmation that the synchronized price covers all carrier/relay/selected speech
charges, and tariff verification within 30 days. Platform execution also requires
enabled billing and Lago configuration. User-owned service costs remain on the
provider account; an optional compatible BYOK lane charges the configured NyxID
price. No new wallet or credential store is introduced.

To renew tariff evidence with the current management API, disable and settle the
binding, revoke its allocation, then allocate the verified resource again with
current tariff evidence and reconnect the specialist. Account trust rotation and
binding re-verification do not renew the tariff timestamp.

SMS replies are bounded to 1,600 UTF-8 bytes and 700 UTF-16 units. A conservative
multipart reservation covers toll-free Unicode segmentation; settlement uses
provider part evidence and releases unused reservations. Answers beyond the
bound receive an explicit shorter-answer request instead of silent truncation.
Accepted, delivered, failed and unknown outcomes remain distinct. A timeout
after a send does not cause an automatic second SMS. Correlated receipts can
recover an accepted send whose HTTP response was lost.

Call and SMS settlement reuse the durable admitted meter and its rates after
credential or price changes. Closing a WebSocket does not establish carrier call
termination: cleanup waits for terminal provider evidence. Pending evidence is
retained for recovery and blocks destructive removal.

## Lifecycle and troubleshooting

**Disable** revokes new execution and starts cleanup. **Verify and enable**
rechecks credentials and provider resources. **Remove connection** requires a
disabled, settled binding. Revoking a platform allocation disables its bindings
and removes them only after active sessions and usage settle. Removing account
trust requires removing its allocations first. Account deletion follows the same
cleanup requirement before removing credentials and owner data. Starting account
deletion closes adapter admission for that owner before checking pending work;
concurrent setup, re-verification and SMS send preparation cannot reopen it.
Authentication remains available while cleanup is pending so the owner can retry
the deletion.

Recovery sweeps check expired sessions and unsettled SMS in bounded batches with
durable progress ordering. They reconcile existing effects; they do not replay
uncertain agent turns or provider commands. Provider credentials may still be
needed to read terminal evidence, so finish cleanup before deleting them.

| Readiness reason | Action |
| --- | --- |
| `disabled` | Enable the rollout and NyxBot access, and verify the active specialist/binding. |
| `connection_required` | Restore an active compatible service connection and its access. |
| `verification_required` | Verify again after credential or trust changes. |
| `resource_not_allocated` | Check ownership claims and the administrator's allocation. |
| `scope_missing` | Permit the required exact operations in the catalog policy. |
| `resource_policy_unresolved` | A future call ID is not yet known; execution will evaluate the actual resource against policy. |
| `billing_unavailable` | Check lane metric/synchronization, tariff freshness and platform billing readiness. |
| `provider_limit_unavailable` | Select a user-owned Telnyx voice connection; platform Telnyx voice is unavailable. |
| `external_speech_billing_required` | Use an external user-owned ElevenLabs speech configuration. |
| `unsupported_operation` | Choose an implemented operation or a different profile. |

Readiness is informational and can change before execution. Exact call-resource
policies are checked when the actual provider call ID exists; a discovery probe
cannot establish permission for an unknown future call.

## Contract and implementation

`GET /api/v1/channel-adapters` returns contract version 1 descriptors from the
executable registry. Existing `GET /api/v1/channel-platforms` retains its channel
bot contract. The `list_channel_adapters` NyxBot account tool exposes capabilities
and the owner setup link. Capability metadata is readable with delegated
`account:read`; binding management and secret delivery require a first-party
human. Lists use `after` and `next_after_id` cursors.

The registry's messaging, conversation and Custom LLM variants dispatch through
`services/call_channels`. HTTP handlers live in `handlers/call_adapters.rs` and
durable records in `models/channel_adapter.rs`. Credentials resolve through the
existing proxy authority path. Guest turns use the existing NyxAgent engine and
specialist tool grants. Provider hosts are fixed, redirects are rejected and
provider effects apply the catalog's canonical operation policy.

The generated frontend fixture
`frontend/src/schemas/fixtures/channel-adapter-catalog.json` is checked against
the full backend registry, including legacy and disabled adapters. Regenerate it
using the `full_registered_catalog_contract_and_legacy_truth` backend test with
`NYXID_UPDATE_CONTRACT_FIXTURES=1`, then run the frontend catalog schema tests.
See the [implementation evidence](plans/2026-10-16-nyxbot-adapters-implementation.md)
and [personal review ledger](plans/2026-10-16-nyxbot-adapters-review.md) for actual
verification results. Controlled provider fixtures do not establish live account
eligibility or production carrier behavior.

Provider protocol references: [Telnyx Conversation Relay](https://developers.telnyx.com/docs/voice/programmable-voice/conversation-relay),
[Twilio ConversationRelay](https://www.twilio.com/docs/voice/conversationrelay),
[Twilio SMS limits](https://www.twilio.com/docs/glossary/what-sms-character-limit),
and [ElevenLabs Custom LLM](https://elevenlabs.io/docs/eleven-agents/customization/llm/custom-llm).

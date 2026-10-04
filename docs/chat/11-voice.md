# Realtime voice for NyxBot and specialists

Status: **Phase 3 merged (PR #1751, v0.54.0); Phase 4 Grok private beta in implementation, flags off**. Initially researched 2026-10-03
against NyxID `240852b5` (`origin/main`, 0.46.0). Implementation validation uses
provider fixtures; no paid provider session has been run. Latencies below are engineering budgets,
not measurements. Provider documentation is evidence of a contract, not proof of
account entitlement or browser acoustic performance.

## 1. Decision

Voice is another surface of the person's existing NyxAgent thread. The voice
model converses, listens, clarifies and narrates; **every operation involving
NyxID, connected services, durable memory, skills or machines runs as a normal
NyxAgent turn using that thread's existing Agent Key**. NyxBot retains Full
authority and specialists retain their grants, B1 operation scopes, B2 skills
and B3a organization checks. Voice creates no second tool executor or general
"call any NyxID API" bridge.

Use OpenAI **`gpt-live-1`**, client delegation, browser WebRTC audio and a
server-owned sideband. Use xAI **`grok-voice-think-fast-2.0`**, pinned rather than
the mutable `grok-voice-latest` alias, through a NyxID WebSocket relay. Both use
one coordinator for transcript persistence, request admission, progress, cards,
Stop and billing. Users may choose an eligible platform or own-key connection;
the browser never receives either provider credential or the thread Agent Key.

Conversation and work have independent lifetimes. A person can keep talking
while work runs; new work is visibly queued. A settled answer is spoken without
requiring another question. Ending voice closes audio and duration billing but
leaves accepted work running in the thread. Stop cancels work explicitly.

Roll out behind the default-off, per-person `assistant:voice` runtime feature
flag. No guests, group/member threads, channel-shared threads or automation
threads in the initial voice surface. Personal NyxBot threads and the acting
person's private specialist threads, including organization specialists, qualify.
This limits the audio surface, not what the agent can do through its existing
tools. Channel and group work remains reachable through NyxBot's ordinary routing.

## 2. Provider contracts and verification limits

The linked official pages were fetched, including their Markdown versions and
the xAI WebSocket schema. Research uses the Live sections of OpenAI's combined
Live/Realtime guides; their Realtime examples are a different protocol.

| Topic | Evidence and design consequence |
| --- | --- |
| Latest documented GPT-Live model | [OpenAI model catalog][oa-models] and [GPT-Live 1][oa-model] list `gpt-live-1`. It is full duplex and uses `/v1/live/sessions`, **not** `/v1/realtime`. The requested `gpt-live-1-mini` page returned 404 and the catalog does not list it. Reserve the preference value, but mark it unavailable until documented and tested; never silently substitute a Realtime Mini model. The claimed September 10 GA date is not needed for this design and was not independently established. |
| OpenAI browser connection | [WebRTC guide][oa-webrtc]: trusted server calls `POST /v1/live/sessions` with `{session:{model,instructions,delegation:{type:"client"},...},transport:{type:"webrtc",sdp}}`; answer is `transport.sdp`, identity is `session.id`. Browser channel label is `oai-events`. Wait for `session.started`; do not send `session.start` again. No ephemeral token is necessary. |
| OpenAI sideband | [Server controls][oa-controls]: attach to `wss://api.openai.com/v1/live/sessions/{session_id}/attach` with the creating project's authentication and required connection headers. It receives subsequent events, including reflected audio. Attach before releasing the answer to the browser; discard reflected audio immediately without persistence. No documented replay guarantee for events before attachment. |
| Browser control permissions | [Live sideband reference][oa-sideband] describes `session.client.data_channel.allowed_client_events` and `allowed_server_events`. Omission allows all. Explicitly restrict them at creation; see §5. Trusted sideband permissions are unaffected. |
| Client delegation | [Delegation][oa-delegation]: `session.delegation.created` carries `offset_ms`, `delegation.id`, `delegation.target`, **no task text**. Reconstruct the request from server-observed transcripts and task state. Append factual context with `session.thinking.append`, speakable facts with `session.commentary.append`, trusted behavior with `session.instructions.append`. Every append requires `delegation_id` (original ID or null) and at most 500 tokens. |
| OpenAI configuration/usage | [Session guide][oa-sessions]: model, voice, input, store and delegation mode are startup choices. `session.update` only updates supported Responses delegation settings, not arbitrary Live configuration. `session.usage.updated.usage.seconds` is cumulative; `session.closed` contains final usage. Appended acknowledgements describe context injection, not speech completion. There is no client-delegation cancel event. |
| OpenAI pricing/storage | [Model][oa-model] lists $0.05/minute billed per second, backend work separate. [Cost guide][oa-cost] says silence, mute and backend waiting count. WebRTC creation bills 15 initialization seconds credited against running duration: a 90-second session is 90, not 105. `store:false` is explicit; no recording download, forking or raw-audio storage. Provider retention policy is separate from NyxID retention. |
| xAI model/transport | [Voice guide][xai-voice] now redirects to Speech to Speech: `wss://api.x.ai/v1/realtime?model=grok-voice-think-fast-2.0`; `grok-voice-latest` currently aliases it. Server Bearer authentication, bidirectional WebSocket, PCM and other codecs, JSON or binary audio transport. |
| xAI tools/updates | Custom functions arrive as `response.function_call_arguments.done`; send `conversation.item.create` with `function_call_output` for every call, then one `response.create`. Wait for current playback to drain to avoid overlapping output. `force_message` synthesizes a scripted line and itself creates a response lifecycle: do **not** also send `response.create`. `interruptible:false` drops caller audio, so routine acknowledgements must stay interruptible. |
| xAI interruption | [Voice reference][xai-ref] and [WS schema][xai-schema] describe automatic interruption with `server_vad`; they do not document OpenAI's `interrupt_response:false`/`create_response:false` switches. Do not assume those work. Use manual response control for the protected laptop-speaker path (§10), with a contract-test gate. |
| xAI transcripts | `conversation.item.input_audio_transcription.updated` is a **replacement cumulative transcript**, including corrections, not a delta. It requires `audio.input.transcription.model:"grok-transcribe"`; `.completed` finalizes it. Do not wait for `conversation.item.done`, which xAI does not emit. |
| xAI idle/resumption | `idle_timeout_ms` prompts repeated check-ins after assistant replies; it is not an inactivity disconnect. Leave it null and enforce NyxID idle close. Resumption is opt-in, replays transcript/tool items using `conversation_id`, expires after 30 minutes idle. Initial release uses `resumption.enabled:false` and restores bounded NyxID text instead. |
| xAI ephemeral direct | [Ephemeral guide][xai-tokens] supports browser `xai-client-secret.<token>` subprotocol. Guide says `session` is unsupported at minting; REST reference says an initial `session` can be bound. Neither establishes an immutable server tool/configuration policy or a Live-style sideband. Resolve this discrepancy before considering a future direct architecture. |
| xAI pricing: open verification | Earlier research found **$0.08/minute plus $0.004/text input** on [models/pricing][xai-pricing], but account-level charges and chargeable injected item types are unverified. Treat these as a pricing question for implementation, not an approved tariff. The WS schema exposes response token counts, not an authoritative cumulative billable-duration event. Duration boundaries, rounding and text-input charges need a controlled trial (§18); token counts are not its voice invoice. |

Use the [Live prompting guidance][oa-prompt] headings for backchannel,
interruption and delegation policy. Identify the selected agent by its real
name/persona, not a second assistant. The prompt permits greetings, clarification,
simple conversation and repeating still-current results; it delegates fresh
facts, connected data, actions and complex reasoning. It says what the backend
can do but contains no raw tool schemas, credential material, skill bodies or
private authorization material. Factual appends are untrusted data, never instructions;
"thinking" is not private storage and may be repeated aloud.

## 3. Existing NyxID contracts and actual gaps

Read alongside [08][engine], [09][nyxbot], [10][uploads], [organization agents][org],
[operation scopes][b1], [agent skills][b2], [platform keys and inference][platform],
[exact billing][exact], `CLAUDE.md` and `DESIGN.md`.

| Existing code | Reuse / required addition |
| --- | --- |
| `services/assistant_nyxagent.rs::begin_turn`, `handlers/assistant_nyxagent.rs::start_turn` | Transactional user-message/key/active-turn admission, detached worker, durable settlement, same upstream session and exact credential. Extract reusable orchestration into the service layer where necessary; a voice service must not depend on HTTP handlers. |
| `handlers/assistant_team.rs::start_server_turn`, `after_turn`, `services/assistant_nyxagent.rs::push_events` | Reuse owner identity, pool accounting, event wakeups and specialist reports. Existing `pending_events` is bounded and channel-aware, not a generic browser FIFO: `/turns` currently returns `turn_active`. Voice needs durable request admission and an explicit atomic bridge to `begin_turn`. |
| `assistant_conversations`, `assistant_messages` | Conversation-scoped sequence allocation, persisted history, active-turn Stop fencing. `via` is already an optional string: add `voice`, not a new channel credential. Add voice provenance and references without changing existing roles. |
| `services/assistant_agent_credential_service.rs` | Exact encrypted thread key, replacement/rotation and authority convergence. Never create a voice-specific all-powerful key. Voice alone need not provision a thread key until delegation; first work uses normal provisioning. |
| `services/org_agent_service.rs`, key auth and proxy/MCP authorization | Live membership, `can_proxy()`, person-private conversations, grant/scope intersection. No personal-resource fallback for org work. Recheck long-lived voice explicitly, not only at the initial upgrade. |
| `assistant_acknowledgement_service`, frontend `use-assistant-nyxagent.ts` | Same action digest, current credential, expiry and one-use consumption. Today some owner-card continuations are browser-owned; voice needs server-owned, deduplicated continuations without also triggering the browser copy. |
| `services/assistant_live.rs` | Metadata-only change stream and owner SSE, invalidation/resync and sweep recovery. Do not put transcripts on its existing owner-wide channel. A session-specific stream carries captions; the task view uses authoritative history. |
| `services/inference_service.rs::default_inference` | `llm-openai` and `llm-xai` have `realtime:true`. It is discovery metadata, not model entitlement, usable credential proof or protocol identification. |
| `handlers/proxy.rs::websocket_realtime_usage_enabled`, `websocket_platform_usage`, direct/node WS pumps | Existing bounded transport and metered lifecycle concepts. The raw passthrough forwards arbitrary frames and cannot enforce voice-tool ownership. Build a constrained adapter, not another public passthrough URL. |
| `services/llm_usage_service.rs::RealtimeLlmUsageCollector` | Deduplicates `response.done.response.id`, sums reported tokens, estimates uncovered response bytes at 4 bytes/token. Reuse real reported token parsing for Grok when relevant; **never use audio/base64 byte estimates to bill voice seconds or invented text tokens**. GPT-Live duration needs a separate collector. |
| `models/service_billing.rs`, `services/billing/owner_resolver.rs` | Integer usage units, exact picocredit money, credential lanes, component pricing, allowance/grant/wallet settlement and ledger. Add a duration unit and durable session checkpoints, not a separate wallet. |
| `components/assistant/chat-composer.tsx`, `upload-composer.tsx`, assistant hooks/schemas | Existing measured composer grid, uploads, send/Stop, thread adoption and actor-fenced caches. Add voice alongside these. Preserve sending attachment-only drafts. |

There is no general successful-browser-turn push notification in the inspected
settlement path. `assistant_team::notify` queues agent events; it is not a human
push. Completion after voice ends therefore needs a small, deduplicated adapter
to the existing notification infrastructure, plus the ordinary persisted reply
and live invalidation. Do not claim background notification already works for
every browser turn.

## 4. One authority path and the meaning of quick

### Admission and identity

Every new route uses `login_client_context::require_first_party_human`, the
human-only assistant router, its OAuth-client rejection layer and the effective
NyxAgent/voice flags. API keys (including assistant keys), service accounts,
delegated tokens, relay tokens and guests cannot start or attach to voice.
Ownership is checked before rate admission, credential resolution or provider
creation. No body accepts an acting user, authority owner, upstream URL, provider
session ID, secret, instructions, generic tool or billing principal.

The session binds the authenticated **acting person**, immutable conversation
ID and agent ID. Organization ownership stays on the agent through the existing
polymorphic `user_id`; no organization transcript or shared org home is created.
Membership never grants another member's speech, history, cards or results.
Live B3a access is checked at start, reconnect, each request/confirmation/result
delivery and on a five-second active-session sweep. Every tool request still
authenticates its thread key normally. Change-stream hints speed revocation but
are not the authority; failed checks fail closed. A previously admitted tool
may finish with its request snapshot under B3a; later output must not leak after
revocation. Stop outstanding turns and close voice on detected access loss.

### Quick path

Choose the simple option for the first release:

* Voice handles conversation, clarification and repetition from already supplied
  context. It cannot fetch data or mutate state.
* Everything touching NyxID or a provider service delegates to the same agent
  turn, including "read my calendar", "remember this" and "turn on the light".
* Session controls (mute/end/Stop/queue inspection) are closed coordinator
  operations. The displayed task list is not a general account-data fast path.
* Disable xAI built-in `web_search`, `x_search`, `file_search` and `mcp`. Otherwise
  voice would bypass the thread's grants, scopes, approval and metering path.

For a short service read, expected normal-turn latency is 1–4 s median and
3–8 s p95 before useful results, plus external service variation. A dedicated
vetted read tool could remove roughly 0.5–2 s of agent routing and model startup,
but would need the same authenticated thread-key constructor, current grants,
B1 canonical operation matching, B3a checks, approval/confirmation, audit and
billing as MCP. Defer it until measured data demonstrates a need. Never use
speculative transcript fragments to execute side effects.

### Specialists and context

Selecting a specialist's private thread speaks as that specialist and runs its
ordinary turns. Starting in NyxBot lets its existing `message_subagent`,
permission requests and `subagent_settled` events route work as today. The voice
model cannot select a more powerful agent or grant itself access. For org
specialists, the thread belongs to the acting member, the skill reader uses
that member's Ornn visibility, and shared org memory is not populated
automatically from private speech. B1 scopes constrain inference operations too;
the model-transport exception does not erase an explicit restriction.

Seed voice with a bounded, role-preserving recap of this authorized thread and
verified task state (at most 20 messages / 8 KiB, also below Live's 128-message /
8,192-token input limits). No other members' history, attachment bytes or hidden
reasoning. Uploads remain accessible to the backend through [10][uploads];
GPT-Live itself has no image modality. A selected attachment must bind through
ordinary message admission and retention checks, never automatically to a new
spoken request.

When a bound NyxAgent session has not seen intervening voice conversation,
supply a bounded, explicitly quoted voice-context delta in its server-built
instructions plus the exact new request in `input`. Track a consumed transcript
watermark so repeats/recaps do not become new instructions. Keep NyxAgent's
existing `conversation` binding and idempotency key; do not invent upstream
fields or use `previous_response_id`. Important missing dates/names/targets
require clarification rather than a guessed action.

## 5. Provider adapters and start/end lifecycle

`assistant_voice_service` owns state and authorization. Provider-specific
adapters accept a resolved, server-only credential and validated configuration;
they produce normalized events:

`Ready`, `UserTranscript`, `AssistantTranscript`, `DelegationRequested`,
`UsageCheckpoint`, `PlaybackState`, `Closed`, `ProviderError`.

Commands are `Start`, `AppendContext`, `Announce`, `SetInputMuted`,
`InterruptSpeech` where supported, and `Close`. Capability data distinguishes
full duplex, native progress appends, output cancellation, final transcripts,
provider duration and supported voices. It is not one fake OpenAI-compatible
event enum. Raw provider frames are never browser-authorized execution requests.

### OpenAI start and control

1. User presses the mic, grants microphone permission, and unlocks output playback.
   Create/adopt a durable draft using the existing `/drafts` flow if needed.
   Prepare one WebRTC offer; keep its outgoing mic track disabled initially.
2. Validate the thread, resolved service/binding/model/voice, quotas and funds.
   Atomically allocate the live-session slot and an idempotent local start record.
3. Create the provider session with `store:false`, client delegation, bounded
   instructions/history, selected `audio.output.voice`, and no `audio.format`.
   Configure `client.data_channel.allowed_client_events` to `[]`: all provider
   commands go through NyxID. Allow only needed transcript/lifecycle/error server
   event types using `{type:...}` selectors; expose no delegated payloads or
   reflected audio there. Verify this capability with the real API before rollout.
4. Save the opaque provider ID server-side, attach the sideband, install all
   handlers, then return the SDP answer. The browser applies it, waits for
   `session.started` and NyxID `voice.ready`, then enables input. Captions may
   arrive directly on the data channel; they are presentation only. The sideband
   is the authoritative source for transcript persistence and delegation.
5. The browser holds a separate authenticated NyxID control socket. Data-channel
   tampering cannot inject tool requests, transcripts, usage or instructions.
   Browser mute disables its track immediately; server mute/unmute waits for the
   corresponding provider acknowledgement. Mute does not end billing.
   The current one-second control tick performs about four reads per live call.
   This is acceptable with one call per person; change-stream push is a future
   optimization, retaining live authorization checks and a polling backstop.
6. End disables capture/playback immediately, requests close through NyxID,
   and drains the sideband/data channel for `session.closed` (five-second local
   finalization budget). Then stop tracks and peer connections. The visible mic
   is off while finalization drains. Unconfirmed finalization is a billing state,
   not a successful zero-cost close.

Session creation is not safely retryable merely because a browser fetch failed.
Local `client_request_id` prevents concurrent duplicates. If an upstream POST's
outcome is unknown, do not POST again automatically; reconcile/close a known
provider ID or report startup uncertainty. Never cache SDP in logs or durable
rows to make this replayable. A replacement connection is a new session/offer.

### xAI relay and why it is selected

Phase 4 implements a separate default-off `assistant:voice-grok` beta and
`assistant:voice-grok-platform` paid-platform gate. Only Hold to talk is admitted;
the UI recommends headphones. Automatic speaker mode is refused server-side
until manual transcription during output and physical loopback AEC are verified.
Provider fixtures exercise the wire contract without making paid calls.

The POST admits an unstarted relay and returns an empty SDP answer; it accepts
no SDP offer. Its human, origin-checked control socket claims the starting session
on the serving replica before resolving credentials again and opening xAI. The
browser sends bounded PCM16 frames and explicit PTT controls, never provider JSON.
The provider actor has bounded channels and processes receipts outside cancellable
timer futures; a coordinator tick cannot abandon a consumed provider event.
Dropping its owner or exceeding the bounded shutdown aborts the relay task and
releases the sole upstream socket. Duration freezes before token settlement I/O.
Shutdown persists pending completed captions and seals incomplete tails without
admitting new work or deciding confirmations. Checkpoints survive cancellation
of the active loop while database or classifier work is in progress.
A recovered Grok lease never reconnects or replays a response: only persisted
elapsed checkpoints are collectible, with the existing 24-hour uncertain-usage
reconciliation deadline. Unknown tail usage remains platform exposure.

The browser connects only to NyxID with first-party human authentication. NyxID
connects upstream with the resolved Bearer key. After `session.created`, configure
the pinned model/voice, manual turn handling (§10), PCM16 mono at 24 kHz,
transcription and **only** a closed `delegate_to_agent` function. Its arguments
reference a server-issued utterance/request identifier and bounded intent hint;
the coordinator uses authoritative input transcript text, not model-authored
instructions, to create the work. Ambiguous references ask for clarification.

Return a durable `{task_id,status:"queued"|"running"}` as the function output
promptly; do not hold the voice response open until the NyxAgent turn finishes.
Fulfil every function call in that response, including bounded refusal outputs
for unknown calls, then request a single response after audio drains. Subsequent
results are **new** context/announcements tied to the NyxID task, not a second
output for an already fulfilled `call_id`. A short fixed `force_message` can
acknowledge admission. Coalesce progress; schedule it and natural result
responses through one output arbiter so speech never overlaps itself.

An ephemeral direct socket saves the extra routing leg but lets the client
choose/update session configuration and own provider events. Current docs do
not establish a trustworthy server-side observer/controller. It would weaken
tool exclusion, metering, revocation and confirmation handling even though the
long-lived key stays secret. The relay adds approximately
`RTT(browser,NyxID) + RTT(NyxID,xAI) - RTT(browser,xAI)` plus buffering to a
round trip: budget 40–150 ms near a suitable region, potentially over 300 ms
cross-region. Measure Singapore, US and EU; deploy regionally if needed.

Use 20–40 ms browser PCM frames and bounded jitter buffers (tune against xAI's
100 ms example); resample explicitly in an AudioWorklet, never assume the device
actually captured at 24 kHz. Keep normalized controls in JSON and audio in bounded
binary frames. The adapter may use xAI binary transport once its response-boundary
association is tested; JSON output with response IDs is acceptable initially.
Never record raw audio. Backpressure terminates or resynchronizes output rather
than accumulating an unbounded delayed conversation.

### Replica ownership and session bounds

One live session per `(acting_person, conversation)` across replicas; additionally
default to one transmitting voice session per person to prevent two tabs from
talking to each other. A conflicting start returns a safe "Voice is already open
for this chat" state; takeover is an explicit End/start action, not silent theft.

Use a MongoDB session lease with generation fencing (refresh every 5 s, expire
after 15 s) and a unique partial live-slot index. Lease loss stops the old
worker's appends, admissions and billing updates and closes its provider connection.
New workers may recover task/settlement state; they do not silently resume audio.
HTTP mutations write durable desired state so the session worker on another
replica sees them. The initial socket binds a worker; reconnect can load state
on any replica and request a new audio session when necessary. Do not rely on
an in-process map or load-balancer stickiness for uniqueness or Stop.

Proposed fixed limits: 30-minute call, or provider `expires_at` if earlier;
3-minute user inactivity with a visible/brief 15-second warning; 30-second
control-heartbeat deadline; 20-second startup deadline; 5 start attempts/minute
and 30/hour/person; 20 control messages/second; 10 pending requests/thread;
same backend per-person turn limits as text. Synthetic silence, provider
heartbeats and the assistant's own progress do not reset user inactivity.
A pending long job can outlive idle closure and still notify. All bounds and
exact user-facing wording need no new environment variable.

## 6. Delegation, progress, interruption and persistence

### Request state machine

`received -> waiting_for_transcript -> queued -> running -> awaiting_confirmation |
completed | failed | cancelled | outcome_unknown`.

1. Deduplicate a provider delegation by `(voice_session_id, provider_event_id /
   delegation_id or call_id)`. Correlate OpenAI `offset_ms` with speaker intervals.
   Allow up to 750 ms for late transcript fragments, with a two-second hard
   deadline; missing/ambiguous text asks the user to repeat and starts no work.
   Never infer a request from assistant output or timing silence alone.
2. Persist the input segment(s) and request mapping transactionally, then
   acknowledge: "I'll check that" or "I've queued that after the current task."
   An optimistic model acknowledgement is not evidence of accepted execution;
   the coordinator corrects it on admission failure.
3. The queue dispatcher rechecks live actor/agent access and calls the common
   turn runner. The claim transaction joins request ID, preallocated turn ID,
   user-message reference and `active_turn`; no second insertion of the spoken
   user message. Existing busy fencing, credential convergence, engine billing,
   Stop and detached execution remain authoritative.
4. User-origin voice work uses `TurnOrigin::User`, not event origin to evade
   limits or consume event-loop quotas. Specialist work still uses the existing
   team pool and reports through the existing parent thread. Explicit work from
   a later utterance queues independently. Repeated delegation of the same
   utterance maps to the same request; do not deduplicate a deliberate repeated
   action solely by normalized words.
5. A metadata-only wakeup, settlement hook and bounded sweep dispatch pending
   work; queue content is stored once as thread messages and referenced by ID.
   Do not stuff voice speech into the channel queue and inherit chat-app
   delivery or grant semantics. Factor shared queue/turn-admission helpers rather
   than introduce another execution engine. Fairness must keep existing event,
   channel and automation work from starving.

At `awaiting_confirmation`, retain the existing card and pending acknowledgement
IDs on the request. Phase 3 performs §11's literal read-back, playback/input
gates and bounded classification. First `unclear` opens one new read-back;
second `unclear` or silence keeps this state without deciding. Approve/deny
creates one server continuation through ordinary admission. Audio end does not
remove a pending card or imply denial.

While work runs, translate allowlisted activity identifiers into short facts:
"Checking your calendar", "Waiting for approval", "The specialist is working".
Unknown labels become "Still working on it". Never narrate arguments, URLs,
tool output, secrets or chain-of-thought. Send quiet state changes immediately;
speak the first meaningful progress after about 2 s, then coalesce to at most
one useful update per 8 s. Avoid repetitive reassurance. Activity status
`completed` does not prove tool success (MCP `isError` can still have that status).

Only durable settlement produces a final result announcement. Persist the full
backend answer; send a bounded, factual speech summary (one/two coherent chunks,
each <=500 tokens for Live) and leave details/cards/attachments on screen.
Do not announce a provider effect as successful from a text delta or a completed
activity alone. If no safe bounded summary is available, say the result is ready
in the chat. Suppress duplicate or superseded announcements using task version
and session generation. Append acknowledgement is not proof the user heard it;
reconnect should show "Result ready" rather than blindly replaying speech.

### Talking over the assistant is not Stop

* "Mm-hmm" and ordinary interruption affect conversational output, not backend
  execution. GPT-Live handles these natively. Grok uses protected barge-in (§10).
* "Stop talking" mutes/interrupts speech only; the task list remains running.
* Unambiguous "stop/cancel that task" invokes the existing durable Stop path for
  the specifically bound active turn, and cancels that task's pending continuation.
  A queued request is removed atomically. Show "Stopping" until settlement.
* With multiple tasks, ask which one or use its on-screen Stop. A stale cancel
  cannot stop whichever unrelated turn happens to be active later: compare the
  request's expected turn ID in the same transaction as Stop.
* "Cancel my booking" is a domain operation and delegates with normal approvals;
  it is not a synonym for stopping NyxAgent execution. Stopping cannot undo an
  already committed remote effect. Report uncertain outcome rather than success.
* A changed date/target is a new request version. Do not silently mutate a running
  turn that may already have acted. Queue the correction, or explicitly Stop and
  wait before starting its replacement; pass the prior outcome to the backend.

Existing composer Stop also invalidates queued voice continuations associated
with the stopped work. "Stop all" explicitly clears this voice session's queued
requests and stops its active work; preserve unrelated schedules and threads.
Stopping NyxBot does not automatically guarantee all already-launched specialists
stopped. Use existing child-task relationships to request their Stop when the
user selected them/all; report any still running. Do not invent a cascade from
speech interruption.

### Transcript correctness

Persist **both** user and assistant speech in `assistant_messages` with
`via:"voice"`. Keep backend answer messages and spoken renditions linked;
display them as one answer with a "Spoken" detail rather than duplicate bubbles.
Live caption deltas are ephemeral; checkpoint grouped segments at least once
per second / 4 KiB, seal on speaker gap or a bounded segment duration (15 s),
and mark incomplete tails on loss. Speech does not have alternating user/assistant
turn boundaries: overlapping intervals are valid.

OpenAI: preserve each speaker's delta text, spaces, repeats, `start_ms`/`end_ms`
and event IDs; use an interval reorder buffer, not packet arrival order. No
imaginary transcript-done event. xAI: replace draft text for an item on `.updated`
and seal on `.completed`; deduplicate final delivery and response IDs. Allocate
the conversation's `seq` transactionally with backend settlements. A late
fragment updates its own unsealed segment; it never rewrites an executed request.

Generated speech is not necessarily heard. Store an output delivery marker
(`generated`, `played`, `interrupted`, `unknown`) and a best-effort playback
watermark. An interrupted transcript is labelled; do not treat unplayed output
as information the user agreed to. Browser playback reports are UI telemetry,
never authorization or the billing clock.

**Required engine adjustment:** background speech/backchannels must not change
the initiating-user-message identity used for acknowledgement dedupe, denial
stickiness or card continuation. Bind each admitted execution to its immutable
input message/seq and use that reference in these checks. Transcript-only rows
are not new authorized backend turns. Audit all existing "latest user message"
and recap consumers, including recaps after key replacement, before permitting
concurrent voice transcript writes.

On voice end, accepted requests keep their queue/turn state and complete into
the same transcript. Use ordinary live invalidation and a completion receipt
to issue one preference-respecting in-app/push notification through existing
notification services, without transcript text in the notification. No new
channel destination is inferred from an old voice call. A voice user message,
like a web user message, clears stale `reply_channel` routing. Resuming voice
requires a user gesture and supplies current task state; it never replays work.

## 7. Sequence diagrams

### OpenAI: start, work, overlapping speech, Stop, card and end

```mermaid
sequenceDiagram
    actor U as Person
    participant B as Browser
    participant V as NyxID voice coordinator
    participant O as OpenAI GPT-Live
    participant N as Existing NyxAgent turn path
    participant D as MongoDB and billing
    U->>B: Tap mic; choose service/key/model/voice
    B->>V: Start(thread, choice, SDP offer, request ID)
    V->>D: Human/owner/B3a/ACL checks; live slot; reserve funds
    V->>O: POST /v1/live/sessions (client delegation, store false)
    O-->>V: Provider session ID + SDP answer
    V->>O: Attach authenticated sideband; install handlers
    V-->>B: SDP answer + local voice session ID
    B->>O: Set remote SDP; WebRTC audio + oai-events
    O-->>V: Session/transcript events
    V-->>B: voice.ready (control socket)
    U->>O: Spoken service request over browser WebRTC
    O-->>V: Input transcript deltas + delegation.created
    V->>D: Deduplicate; persist input and request; admit/queue
    V->>O: commentary.append (accepted/queued, delegation_id)
    O-->>U: Immediate acknowledgement
    V->>N: Ordinary user turn on same thread/key
    N-->>V: Allowlisted activity state
    V->>O: thinking.append / coalesced commentary.append
    O-->>U: Brief useful progress
    U->>O: Backchannel, interruption or another request
    Note over O,N: Full-duplex speech continues; interruption leaves work running
    O-->>V: New transcript/delegation
    V->>D: Queue new work behind active-turn fence
    N->>D: Settle result durably
    V->>O: commentary.append (settled result)
    O-->>U: Proactive result
    opt Explicit cancel of another running task
        O-->>V: User transcript identifies task cancellation
        V->>D: Expected-turn Stop; invalidate task continuation
        V->>N: Existing Stop/drop-stream and machine cancellation
        N->>D: Persist cancelled or uncertain outcome
        V->>O: commentary.append (verified cancellation state)
    end
    opt Next task needs a one-use action card
        N->>D: Pending acknowledgement with exact action digest
        V-->>B: Optional card record + pending state
        V->>O: Read exact server summary and ask confirmation
        V->>V: Wait for output transcript completion and playback watermark
        U->>B: Speak a decision after the question
        O->>V: Sealed authoritative input segment on sideband
        V->>V: Reject overlap/echo; bounded stateless classifier
        V->>D: Recheck exact card, key, expiry and live org authority
        V->>D: Decide card and record one continuation receipt
        V->>N: Server-owned ordinary continuation
    end
    U->>B: End voice
    B->>V: End (local capture/playback off immediately)
    V->>O: session.close
    O-->>V: session.closed with final usage.seconds
    V->>D: Settle duration; release session slot
    V-->>B: voice.ended; close media/control transports
    Note over N,D: Accepted work survives audio end; reply/notification stays in thread
```

### Grok: relay, asynchronous function receipt, interruption, card and end

```mermaid
sequenceDiagram
    actor U as Person
    participant B as Browser audio engine
    participant V as NyxID voice coordinator/relay
    participant X as xAI voice
    participant N as Existing NyxAgent turn path
    participant D as MongoDB and billing
    B->>V: Start private thread + authenticate control/audio socket
    V->>D: Human/owner/B3a/credential ACL; reserve slot and funds
    V->>X: Bearer-authenticated pinned-model WebSocket
    X-->>V: conversation.created / session.created
    V->>X: session.update (manual control, delegate function only)
    X-->>V: session.updated
    V-->>B: voice.ready
    U->>B: Speak
    B->>V: Bounded PCM + local speech boundaries
    V->>X: input_audio_buffer.append / commit
    X-->>V: Input transcription completed
    V->>X: response.create
    X-->>V: function_call_arguments.done + response.done
    V->>D: Bind transcript; persist and admit/queue task
    V->>X: function_call_output (task ID, accepted status)
    V->>X: response.create once all outputs sent and playback drained
    X-->>B: Acknowledgement audio via NyxID and echo-referenced player
    V->>N: Normal same-thread turn
    N-->>V: Activities
    V->>X: Scheduled short progress/context at safe speech boundary
    X-->>B: Progress audio
    U->>B: Talk while output/work continues
    B->>B: Duck/pause output on candidate speech, retain bounded tail
    B->>V: New input audio; no automatic cancel
    V->>X: Commit for transcription while manual output is active
    X-->>V: Confirmed real user transcript
    V->>X: response.cancel + truncate to played audio if interrupted
    V->>D: Queue a new request OR expected-turn Stop for explicit cancel
    Note over N,X: Speech cancel does not cancel NyxAgent; explicit Stop does
    N->>D: Durable result / cancellation settlement
    V->>X: New result context + one scheduled response.create
    X-->>U: Natural result announcement through browser player
    opt Action card confirmation
        N->>D: Pending one-use acknowledgement
        V-->>B: Optional card record + pending state
        V->>X: Read exact server summary and ask confirmation
        V->>V: Wait for output transcript completion and playback watermark
        U->>B: Speak a decision after the question
        X->>V: Completed authoritative input transcription
        V->>V: Reject overlap/echo; bounded stateless classifier
        V->>D: Recheck exact card, key, expiry and live org authority
        V->>D: Decide exact card and record one continuation receipt
        V->>N: Server-owned ordinary continuation
    end
    U->>B: End
    B->>V: End; stop mic/player
    V->>X: Close upstream WebSocket; drain bounded final events
    V->>D: Final server duration checkpoint; settle; release slot
    Note over N,D: Accepted tasks continue; no socket-triggered replay
```

## 8. Data model and routes

All additions are proposed. New model documents use UUID-v4 string `_id`,
`COLLECTION_NAME`, BSON UTC datetime helpers (including optional dates) and
redacted Debug. No `skip_serializing` model fields. Use dedicated API DTOs and
strict handlers -> services -> models layering. Keep monetary values on existing
exact-money types and storage paths.

| Storage | Proposed fields and invariants |
| --- | --- |
| `assistant_voice_sessions` (new) | Acting `user_id`, `conversation_id`, `agent_id`, resolved service/connection ID, credential class/reference and binding revision, adapter/model/voice, state (`starting`, `active`, `closing`, `closed`, `failed`), `live_slot`, local `client_request_id`, worker lease/generation, created/ready/last-user/closed times, hard deadline, safe end reason. Provider session ID is server-only protected metadata; never a client-chosen locator. No credential copy, SDP, ICE, audio or transcript body. Unique `user_id` where `live_slot:true`; idempotency `(user_id,client_request_id)`; expiry/reconciliation index. Clear slot only through fenced closure/recovery. |
| `assistant_voice_requests` (new admission records) | User/thread/session, provider delegation/call ID, immutable source-message IDs and transcript cutoff, request version, dedupe identity, preallocated backend turn ID, queue seq, state, queue expiry, `awaiting_confirmation` state, pending acknowledgement IDs, continuation ownership, result-message ID, notification/announcement receipt. No duplicated prompt/tool payload. Unique provider-delegation identity and `(conversation_id,queue_seq)`. A transaction claims exactly one request into the existing turn fence. |
| `assistant_messages` (additive) | Reuse `via:"voice"`; optional `voice:{session_id,segment_id,speaker,start_ms,end_ms,revision,sealed,delivery,request_id,backend_message_id}`. Existing `turn_id` remains a string: transcript-only segments use a separate UUID grouping ID and are explicitly not admitted execution turns; claim associates request input with its preallocated execution turn. DTOs make that distinction explicit. Deduplicate segments by session/speaker/segment identity; use existing conversation seq allocator. No raw audio attachment. |
| `ActiveTurn` / turn admission (additive) | Optional immutable initiating message/seq and voice request ID. Claim validates all referenced input rows belong to the actor/thread and are unclaimed. Scope prompt deltas to a consumed voice watermark; preserve on continuation/reset. Ordinary text writes retain current semantics. |
| `assistant_settings` (additive sibling) | Optional `voice:{service_id,connection_id,key_source,model,voice?,input_mode,language?,notify_on_completion}`. Defaults select `gpt-live-1` only if usable, otherwise show available providers without auto-start. Missing fields preserve current settings. Server stores preferences, not grants; revalidate every start. Voice choice changes require a new session where the provider requires it. |
| Existing usage rows + voice checkpoints | Persist session cumulative observed/settled units, initialization debit status, fixed window IDs, reservation references, finalization/provenance (`provider`, `server_clock`, `unconfirmed`) and checkpoint fencing. Use normal usage settlement/ledger tables; no separate balance. Keep enough metadata after session cleanup to reconcile money. |

TTL is never responsible for releasing a live lease or deleting an unsettled
money record. Terminal session/request metadata may expire after 30 days only
once settled; request content already lives in normal conversation history.
Conversation deletion/account purge closes sessions, cancels
queued work and deletes speech/session references through existing
cascades; legally retained billing rows contain metadata only. Live voice is
closed before deleting its thread. An active backend turn retains the existing
409 deletion rule until stopped/settled.

Paths below are relative to `/api/v1/assistant/nyxagent`. All are human-only,
actor-scoped, flag-gated on creation and reauthorized on use. Enforcement,
cleanup and Stop continue when the feature flag is disabled.

| Route | Closed request / safe result |
| --- | --- |
| `GET /voice/options?conversation_id=...` | Authorized realtime services and valid connection/binding choices, adapter, verified models/voices, availability reason, lane prices and billing owner label. Metadata only, no credential provisioning side effects. |
| Existing `POST /drafts` | Create a normal private thread for selected agent before the first call if necessary. |
| `POST /conversations/{id}/voice-sessions` | `{client_request_id,preferences:{service_id,connection_id?,key_source,model,voice,input_mode,language?,notify_on_completion},sdp_offer}`. OpenAI SDP required, xAI forbidden. Returns `{session,sdp_answer}`; the safe session DTO carries state, timing and revision, and options disclose limits. It does not change the thread's NyxAgent profile. |
| `GET /conversations/{id}/voice-sessions/{sid}` | Authoritative safe state and task IDs for reconnect. No provider IDs, SDP, key IDs or transcript dump. |
| `GET /conversations/{id}/voice-sessions/{sid}/stream` (WS) | Human control socket and normalized captions/task state; xAI audio additionally flows here. Authenticate before accepting input or making an upstream connection. No bearer/query tokens in the URL. |
| `POST /conversations/{id}/voice-sessions/{sid}/control` | Idempotent `{command_id,expected_revision,action:"mute"|"unmute"|"end"}`; durable desired state, safe status. Used by ordinary HTTP controls and cross-replica recovery. |
| `POST /conversations/{id}/voice-requests/{rid}/cancel` | Empty body; idempotent by request ID. Server resolves expected active turn and performs normal Stop or queued cancellation. No caller-supplied tool/authority. Existing `/stop` remains available. |
| `GET /conversations/{id}/voice-requests/{rid}` | Owner-authorized request state, turn ID and pending acknowledgement IDs; remains readable after flag disable. |
| Existing acknowledgement route | Clicking Allow/Deny is an optional fallback racing the spoken decision path. Voice-associated decisions return `continuation_owner:"server"` and a receipt, so the upgraded frontend does not also send a continuation. Server rechecks existing exact-card policy. |
| Server-only spoken decision adapter (Phase 3) | No client-supplied decision/transcript endpoint. After §11 playback/input gates and bounded classification, call the existing acknowledgement service with source `voice`; accept only authoritative provider segments. |
| Existing `GET/PUT /settings` | Optional bounded voice preference object; omit preserves, explicit null resets. No product-wide setting or secret input. |

Phase 2 mounts options, the unavailable start stub, request status/cancel and
preferences only. Options mark adapters unavailable, and start cannot create a
provider session even with the flag enabled. Session sockets/control and spoken
decisions land in Phase 3. Cancellation is already idempotent by request ID;
it needs no command body until session control exists. The existing 15-second
sweep is the durable dispatch backstop; adapters add immediate dispatch in Phase 3.

Add explicit route tests to `delegated_read_denied_path` for the new WS stream
and private/session GETs before mounting; `/assistant` is already a denied class,
but keep regression coverage. Check Origin/CSRF for cookie-authenticated starts
and upgrades, validate WebSocket subprotocol/version and reauthenticate when
the human session expires. If browser deployment uses header-only auth, add a
single-use, 30-second actor/thread-bound socket ticket via authenticated POST,
consumed before audio/provider start in a first frame; no long-lived bearer in
URL/subprotocol/logs. Reuse the existing human socket pattern where applicable.

Bound start JSON to 128 KiB (SDP <=64 KiB), non-audio control frames to 16 KiB,
provider metadata frames to a separately bounded 256 KiB, PCM frames to 100 ms
of the negotiated format, transient PCM rings to <=2 s and UI replay to a
bounded event window. Rate-limit bytes and concurrent sockets as well as message
counts. Malformed recognized events fail closed; unknown harmless events are
ignored/count-only. Do not expose upstream error bodies or allocate new numeric
error codes in this design; use stable safe voice error names and assign codes
centrally in `errors/mod.rs` during implementation.

## 9. Event mapping

### OpenAI adapter

| Provider event/command | NyxID mapping and rule |
| --- | --- |
| HTTP create result; `session.started` | Bind server-owned provider session; `voice.ready` only after sideband and control readiness. Deduplicate start notifications. |
| `session.input_transcript.delta` | Timed user caption/segment; authoritative only on sideband. Correlate with delegation offset; never generic confirmation. |
| `session.output_transcript.delta` | Timed assistant caption/segment; delivery tracked separately. |
| `session.delegation.created` | Idempotent request admission using stored input transcript and known task state. |
| `session.thinking.append` | Bounded factual quiet task state; may later be spoken, no secrets/reasoning. |
| `session.commentary.append` | Accepted/queued/progress/result/cancellation fact, original delegation ID or null for unrelated session state. |
| `session.instructions.append` | Application-authored behavior only; never verbatim user/tool text. Can steer speech; cannot undo audio or cancel work. |
| `session.*.appended`, `client_event_id` | Track command acceptance/context injection; not task completion or playback receipt. Errors mark delivery uncertain, never rerun work. |
| `session.input_audio.mute/unmute`, `.muted/.unmuted` | Server input gate in addition to local mic gate. Does not mute output or stop billing/tasks. |
| `session.usage.updated` | Monotonic cumulative seconds checkpoint; never sum cumulative snapshots. |
| Reflected input/output audio on sideband | Drop without retaining; no record/replay or usage estimation from these bytes. |
| `session.close` -> `session.closed` | Stop new delegation, drain final usage, mark end reason, settle once. Backend tasks continue unless separately stopped. |
| Sideband `error` / missing close | Static safe error, incomplete finalization, bounded recovery; no automatic provider-create retry. |

### xAI adapter

| Provider event/command | NyxID mapping and rule |
| --- | --- |
| `conversation.created`, `session.created`, `session.updated` | Server-only provider identity/config; readiness after validated acknowledgement. |
| `input_audio_buffer.append` / binary audio | Adapter-generated from bounded authenticated mic input; never raw browser JSON forwarding. |
| `input_audio_buffer.commit` | Manual endpoint from speech gate/PTT; requires `turn_detection: {"type": null}`. |
| `input_audio_buffer.speech_started/stopped` | VAD hints in supported automatic mode; never task cancellation or confirmation authority. |
| `conversation.item.input_audio_transcription.updated` | Replace draft text keyed by item ID; requires `grok-transcribe`. |
| `conversation.item.input_audio_transcription.completed` | Final user text and request boundary; perform echo/intent checks before accepting work. |
| `response.function_call_arguments.done` | Validate function name, closed arguments, call/response identity and transcript reference; persist task before receipt. Unknown tools get a safe refusal. |
| `conversation.item.create(function_call_output)` | Exactly one receipt for each provider call; fulfil the whole batch, then request one response. |
| `response.create` | Coordinator output arbiter after inputs/tool outputs and playback drain; no concurrent response storm. |
| `response.output_audio.delta` / supported `response.audio.delta` | Audio buffer keyed by response/item; stale cancelled generations discarded. |
| `response.output_audio_transcript.delta/done` (compatible aliases normalized) | Assistant captions; final transcript separate from played audio. Contract fixtures pin actual deployed names. |
| `response.done` | Response terminal event, not playback completion. Reuse reported-token collector only if token capture applies; it is not duration. |
| `response.cancel`; `conversation.item.truncate` | Cancel speech after verified interruption, truncate to played milliseconds. Never Stop backend implicitly. |
| `conversation.item.create(force_message)` | Scripted accepted/queued/error/progress line, interruptible; no following `response.create` for that line. |
| `input_audio_buffer.timeout_triggered` | A provider check-in, not user input or a backend request. Disabled initially. |
| WS close / error | End server duration clock, reconcile receipts, safe error. No invented `session.closed` usage contract. |

Normalized browser events carry session generation, monotonically increasing
local cursor and bounded fields: `voice.ready`, `caption.updated`, `caption.sealed`,
`task.queued`, `task.started`, `task.progress`, `task.awaiting_confirmation`,
`task.settled`, `voice.muted`, `voice.warning`, `voice.ended`, `voice.resync`.
On gaps, fetch session/task/history state; do not resend audio, confirmations or
work. Existing owner live events retain their metadata-only schema.

## 10. Echo, noise and turn-taking

### Capture and playback choice

Request `getUserMedia({audio:{echoCancellation:true,noiseSuppression:true,
autoGainControl:true,channelCount:1}})` with feature detection and inspect
`getSettings()`. Constraints are requests, not proof of successful cancellation.
Use one capture stream, avoid feeding microphone monitoring to speakers and
reset AEC after device changes. Never use `MediaRecorder` chunks as the realtime
PCM pipeline or send compressed bytes labelled PCM.

[W3C Media Capture][aec-spec] and [MDN][aec-mdn] define the portable reference:
`remote-only` cancels incoming audio tracks sourced from `RTCPeerConnection`;
boolean true must attempt at least that, while broader local/system cancellation
is implementation-dependent. Therefore a local WebAudio oscillator/buffer or
`MediaStreamAudioDestinationNode -> <audio>` alone is not a portable guarantee.

| Browser family | Evidence and selected path |
| --- | --- |
| Chrome/Chromium | [Chrome's native AEC discussion][aec-chrome] documents device/platform-dependent cancellation; it is an older implementation article, not proof that current arbitrary WebAudio playback is cancelled. Use WebRTC receive audio. Do not ship experimental `echoCancellationType` flags. |
| Safari/macOS/iOS | [WebKit's WebRTC/media capture guidance][aec-webkit] supports the WebRTC capture/playback architecture and documents user-interaction playback constraints. It does not guarantee cancellation for locally generated WebAudio. Unlock `<audio>` and AudioContext from the mic gesture; validate physical iPhone and Mac playback. |
| Firefox | Rely on standards-based peer-connection receive tracks and the runtime capture settings. Retrieved cross-browser documentation does not establish that Firefox cancels every arbitrary WebAudio source; treat that as unproven. Physical-device QA is required here too. |

OpenAI: render the actual WebRTC remote audio track in an `<audio autoplay
playsinline>` element. Keep that playback path intact; a waveform analyzer must
not replace it with a second speaker path.

Grok: PCM -> AudioWorklet jitter queue -> `MediaStreamAudioDestinationNode` ->
**local loopback pair of RTCPeerConnections** -> receive-track `<audio>`.
Only the receive side plays; the worklet is not connected to AudioContext's
speaker destination. Exchange local ICE in memory, no external STUN/TURN and
no server audio hop for this loopback. This creates the WebRTC receive reference
the standard describes, at an estimated 20–60 ms additional delay. Test local
candidate/privacy restrictions, suspended contexts, sample-rate conversion and
AEC on each browser; standards language alone does not certify acoustic results.
If the path fails, use headphones/PTT and explain it. Do not label an unverified
raw WebAudio fallback as echo-cancelled. Revisit a native browser-to-relay WebRTC
audio transport only if these measurements require it; that is additional scope.

### Grok false barge-in gate

Default `server_vad` can stop generation before NyxID verifies speech. Ducking
only in the browser cannot undo that. The intended protected path is
`turn_detection: {"type": null}` with local VAD/endpointing and NyxID-controlled response
creation/cancellation:

1. While output plays, a candidate voiced input ducks output by roughly 12 dB
   (or pauses with a bounded retained tail). Never cancel a response or task from
   a VAD event alone.
2. Continue capture/transcription. Commit a candidate utterance in manual mode;
   wait for a real input transcript. Combine voiced duration, output-reference
   correlation and transcript evidence; identical speaker leakage, cough/noise
   or a short backchannel must not hard-cancel. Transcript text alone is not
   sufficient because echo can itself produce real words.
3. A meaningful user interruption cancels provider output, flushes stale queued
   audio and truncates the provider conversation to the actual played position.
   A rejected candidate restores output promptly. No task state changes unless
   the input explicitly requested Stop/correction/new work.
4. Contract-test that Grok manual mode can transcribe committed input while an
   assistant response is generating without auto-cancelling it. If unsupported,
   keep automatic speaker mode disabled; this effort adds no separate ASR
   dependency. Do not ship `server_vad` as meeting this guarantee. Ship a labelled
   PTT/headphone beta first and gate automatic mode on provider transcription
   and loopback AEC evidence.

The Phase 4 PTT gate measures committed voiced samples, checks a bounded output
PCM reference and uses the normal billed, tool-less one-shot helper to classify
meaningful interruption versus backchannel/uncertainty. No lexical allow/deny
lists are used. Fixtures substitute that advisory classifier. A provider VAD
event cannot cancel speech or backend work. Truncation retains the provider item
after `response.done` so buffered playback can still be interrupted; cancelled
output transcripts cannot arm a spoken confirmation. Browser fixtures verify
resampling, one receive-track speaker path, bounded buffers and cleanup. Separate
capture/playback generations reject worklet messages delayed across PTT holds or
output flushes; stale PCM cannot enter the next utterance and stale playback marks
cannot advance a later readback. The worklet is a separate same-origin asset.
These fixtures do
not establish physical AEC performance or live-provider manual-mode behavior.

GPT-Live handles backchannels and duplex interruptions natively. Do not layer
a half-duplex output cancellation rule over it. AEC still matters; its full-duplex
model is not an acoustic echo canceller.

Neither fetched GPT-Live session schema nor xAI's published voice schema
documents the Realtime `input_audio_noise_reduction:{type:"far_field"}` control.
Do not copy that field from OpenAI Realtime into Live or xAI. Enable laptop
far-field reduction only if the selected adapter's actual API documents and
accepts it. Browser noise suppression is the baseline.

Offer Mute, a hold/toggle push-to-talk mode, input-device selection where
supported, and an optional short echo self-test: play a neutral fixed phrase
through the real output path while the user stays silent, score echo locally,
then ask for a real interruption. Store only pass/fail/device-class metadata,
no recording. Suggest lower volume, headphones or PTT on repeated failures.
Mute/end must work locally even if NyxID or the provider is unavailable.

## 11. Spoken confirmation: server-enforced read-back

Voice confirmation is a two-turn exchange. The existing one-use acknowledgement
remains the authority and transcript record; its on-screen card is an optional
fallback. Voice users never need to look at or touch the screen. Respect existing
`skip_destructive_confirmation` behavior and mandatory machine/webhook policies.
There is no private-code challenge, challenge collection/route or local ASR.

When a delegated turn raises a card, mark its voice request
`awaiting_confirmation` and bind the pending acknowledgement IDs. NyxID supplies
the **server-generated card summary**, including the action and key details,
for literal read-back: “Delete the repository nyxid-demo — should I go ahead?”
The voice model must not reconstruct or paraphrase tool arguments. Task execution
remains paused at the existing one-use gate while conversation can continue.

Open a decision window only after authoritative output-transcript completion
**and** the playback watermark show that the whole question was spoken. Bind
that boundary to the exact card, output item, session generation and playback
clock. Provider append acknowledgement or audio generation completion alone is
insufficient. The window lasts at most 60 seconds and never exceeds card expiry.
A client watermark can only delay admission; Phase 3 must establish a trusted,
monotonic mapping to server-observed output and must fail closed on gaps/reconnect.
Only sealed authoritative input segments whose speech starts strictly after
this boundary qualify: GPT-Live sideband input transcript, or xAI completed input
transcription. Segments overlapping any assistant playback are discarded.
Discard segments whose normalized text is contained in nearby assistant output,
including an echoed “yes”: require
`user.start_ms < output.end_ms + playback_lag_ms + 1500` and
`user.end_ms > output.start_ms`. Temporal playback overlap always
rejects input; containment alone cannot reject a reply matching older speech
(for example, “yes” spoken 20 seconds earlier). Bound and expire the output
comparison buffer; do not persist it or use lexical approve/deny lists. AEC remains necessary; these guards add to
§10's Grok manual-transcription/loopback gate, without claiming unsupported VAD.

NyxID classifies the **user's utterance**, against this exact action, through a
bounded, stateless one-shot model call using only the server summary and sealed
user transcript as untrusted data. Reuse the U1 title-generation inference helper
(`services/assistant_oneshot_inference.rs::one_shot_text`), the person's resolved
inference credential and normal billing.
Require a closed `approve | deny | unclear` response, no tools/history/state,
fixed input/output limits and timeout; failure or malformed output is `unclear`.
There are no hardcoded word lists. A conversational model claim, function call,
assistant transcript or tool/backend output can never enter this decision path.
The classifier proposes a decision; NyxID rechecks all temporal and authority
fences after inference and before applying it.

`approve` and `deny` use the existing acknowledgement decision service and its
current-key, action-digest, expiry, live org access and one-use checks. The exact
card decision and server-owned continuation receipt commit atomically. A click
on the optional card races through that same path: exactly one decision and
continuation wins. The receipt suppresses the browser's continuation. Stop before
claim, key rotation, access loss or card expiry prevents stale continuation.
Audit source `voice` with actor/thread/request/card/receipt IDs only; no summary,
utterance, tool arguments, audio or classifier payload in audit or logs.

On first `unclear`, ask the same read-back once more and establish a new playback
boundary. Second `unclear` or silence closes that attempt, announces once that
the action is still pending, and leaves the card undecided. A later user request
in the call may begin a fresh read-back; it does not reuse an expired window.
A spoken refusal is a denial. Ending audio leaves pending cards in the thread.

Phase 2 includes pending-card state and atomic continuation receipts. Phase 3
ships playback/input correlation, echo guards, the U1-based classifier and the
spoken decision adapter; no classifier or provider media is enabled in Phase 2.

The Phase 2 bridge stores pending acknowledgement IDs on each voice request.
The existing decide transaction creates its deduplicated continuation request
and returns `continuation_owner: "server"` plus the receipt ID, so a browser
does not start a second turn. Claim rechecks the stored input, parent cancellation
and current key through ordinary turn admission. Bounded recovery closes abandoned
claimed requests without replaying execution. It never treats expiry as approval.

## 12. Credentials, billing and financial recovery

### Discovery and choice

List only caller-visible, active services with validated `inference.voice` metadata
intersected with compiled, supported adapter protocols (OpenAI Live in Phase 3). Batch through existing service visibility, `OwnerGrants`,
provider status and connection resolvers; no per-option membership query and
no across-request grant cache. Distinguish catalog metadata, usable binding,
model entitlement and temporary provider health. A custom service marked
realtime does not automatically become an OpenAI/xAI transport.


`ServiceInference.voice` is additive and serde-defaulted: `protocol` (`openai_live`
or `xai_realtime`, with unknown future protocols ineligible for execution), `models`
(`id`, `label`, optional `default`), `voices` (`id`, `label`), `usage_source`, and
`billing_metrics`. Compatibility `realtime` is projected true whenever voice is
present. Response-only `capabilities.supports_realtime_voice` derives from the
same metadata in catalog, keys and MCP; it is never independently stored.

Startup seeds OpenAI `gpt-live-1` and xAI `grok-voice-think-fast-2.0`, plus their
documented voice IDs, without any prices. A separate null-guarded update fills
only `inference.voice` on existing inference blocks. Both seed paths skip
`inference_admin_modified:true`, including whole-block and voice null clears.
Admin edits preserve this marker. Unknown fields remain readable by old replicas.
IDs/labels are bounded to 128 bytes, model lists to 32, voices to 64, metrics to
six unique duration/token units; IDs cannot contain URLs and at most one model
is default. The first listed voice is the fallback when no voice is selected.

The existing service Inference editor owns these choices. Adding an officially
verified Mini later requires only a catalog edit. Options and start validate the
catalog choices again; no runtime model-name or voice-name allowlist exists.
Adapters retain fixed origins and protocol/usage-source validation. Metadata alone
never enables an unshipped adapter or overrides the paid platform rollout gate.
Call setup displays the resolved model/voice, credential class, tariff and payer.

Platform resolves through the existing shared platform ACL **before decrypting**
the catalog master credential; fixed catalog destination, no user headers,
gateway URL, credential-bearing redirects, user-selected host or node routing.
Own key resolves the chosen active connection through ordinary credential
classification and epoch handling. Initial adapters require server-resolvable
credentials and supported official/provider-approved destinations; node-held
keys and arbitrary compatible gateways are explicitly unavailable until an
adapter can enforce the same authority. Never silently fall back to platform,
another person's connection or a more expensive model.

`key_source` is a selector, not an arbitrary injection override. Resolve a
connection whose stored `credential_binding` matches it. If the requested own
or platform variant is not established, offer the existing connect/edit flow
using `use_platform_key`; only an explicit reviewed connection change goes
through `unified_key_service`. Switching a call preference must not silently
rewrite the default binding of a shared connection. Switching to platform
retains the stored own key exactly as existing rules require. Remember the
chosen connection/source in assistant settings, then validate again next call.

The voice-provider model transport uses the narrowly bounded assistant
inference access policy; it cannot open service tools. Apply any configured B1
scope to the canonical provider control operations too. B1 currently denies
WebSocket execution on a scoped service, including inference: a restricted
Grok service must therefore be reported unavailable rather than bypassed via
the relay. OpenAI's HTTP create plus WS attach also needs that check. Do not
quietly translate an HTTP-only operation grant into an unrestricted live channel.

Call `BillingOwnerResolver::resolve_for_execution` **after** final credential
classification. Platform (`NyxidManagedMaster`) always bills the acting person,
even when an org grant permits it. Personal own key bills that person; authorized
org BYOK retains existing org-wallet billing. "Same billing person" identifies
the actor, not a new rule overriding existing resource-owner BYOK accounting.
Show the payer and both relevant prices before starting. NyxAgent/model/tool
work retains its own normal execution billing independently of voice duration.

### Duration accounting in existing lanes

Add `BillingMetric::VoiceSeconds` / `voice_seconds` and a defaulted
`PlatformUsage.voice_seconds` integer counter. Update centralized Rust metadata,
frontend `schemas/billing-metrics.ts`, CLI unit handling, allowance metrics,
rollups, rate cache, Lago sync, component settlement and API schemas together.
UI displays seconds/minutes; money remains exact `Credits`, Decimal128 BSON and
decimal strings. No floats or minute rounding in credit movements.

Add a synced duration component to the applicable BYOK/platform lane while
preserving existing text-token prices. A voice call reports zero token quantities
unless actual provider-reported tokens have configured components; it must not fall through
to byte-estimated token billing. Ordinary text requests report zero seconds.
Own-key voice without a `byok_pricing` lane, or with a synced primary but no
`voice_seconds` primary/component, is allowed as metering-only: record seconds
without charging allowances, grants or wallets, even when legacy platform pricing
exists. These options return no voice quote. The UI discloses no NyxID voice charge
and that provider charges still apply. An existing BYOK primary must be synced;
an explicitly configured duration price must also be synced. These voice admission
rules do not change ordinary text billing or its configured token prices.

| Credential and lane state | Voice admission and duration billing |
| --- | --- |
| BYOK: no lane | Metering-only; no voice charge or quote. |
| BYOK: synced primary, no `voice_seconds` primary/component | Metering-only; no voice charge or quote. |
| BYOK: pending/failed primary | Refuse. |
| BYOK: pending/failed `voice_seconds` primary/component | Refuse. |
| BYOK: synced primary and duration price | Quote and charge the configured duration price. |
| Platform | Require synced primary and duration price plus the separate operator flag; quote and charge normally. |

Initial service prices are per-provider tariffs, as the current lane schema is
per service rather than per model. Do not invent model-specific price lookup.
Until a reviewed model-price extension exists, all enabled voice models on a
service share the displayed seconds tariff; differing model costs are an admin
pricing decision. Never advertise an unverified Mini discount.

* **GPT-Live:** cumulative `usage.seconds` from the authenticated sideband is
  primary. Persist the maximum confirmed checkpoint; subtract already-settled
  units, never sum snapshots. Reserve initialization funding before creating a
  session. Reconcile the documented 15-second initialization debit as a credit
  against duration, not an additional 15 seconds. Verify short calls, failed
  handshakes and initialization-only usage in an entitled account before paid
  rollout; the guide does not fully specify these boundary invoices.
* **Grok:** use the server's monotonic elapsed duration from successful provider
  connection/configuration to confirmed upstream close, including silence,
  waiting and mute. Persist elapsed milliseconds for recovery/provenance. The approved
  NyxID tariff is completed seconds per session (`floor(total_ms/1000)`), with
  subsecond residue carried across checkpoints and the terminal fraction waived.
  This is an explicit NyxID tariff, not a claim about xAI invoice rounding.
  Validate the upstream billing boundary in the provider trial.
* Preserve exact decimal provider-reported durations in metadata if fractional;
  provisional settlement uses completed seconds and the same disclosed terminal
  fraction policy. Never round independently per audio chunk/window. Initialization
  minimum accounting is provider-specific and not applied to Grok.
* A separate Grok text-input fee remains an unresolved pricing question. If a
  confirmed fee is not represented in authoritative token usage, the admin can
  fold it into the disclosed seconds tariff. Actual reported tokens use configured
  token components; never synthesize a second token count, duration quantity or
  per-frame request charge for the same usage. No new text-input money path or
  dedicated metric is introduced here. Keep context appends bounded.
* Reuse `RealtimeLlmUsageCollector` for actual Grok `response.done` token
  provenance and configured token-component settlement, keyed by response ID.
  Duration is settled only from the duration checkpoint; token events carry zero
  seconds, and repeated response IDs reuse deterministic settlement identities.
  xAI has no provider duration event: its token counts must never become a duration
  estimate. Implement this adapter path in Phase 4. Disable uncovered-byte estimation on
  this voice surface. GPT-Live client delegation has no Responses backend to
  double-charge; NyxAgent work is already metered on its normal path.

Grok reserves tokens through the existing billing service before each response
or scripted utterance, using a deterministic session/response-sequence identity.
The actor binds one provider response ID to that reservation and deduplicates
terminal events before persisting the ordinary settlement intent. Actual token
settlement uses only provider-reported counts; byte estimates apply only to the
reservation budget, never the final charge. Token rows carry zero seconds and
duration rows carry zero tokens. A text-only BYOK lane may therefore meter voice
seconds freely while still disclosing and charging its configured reported-token
components. A missing terminal report remains uncertain exposure; no estimated
tokens are invented on close.

### Reserve before spending; settle once

Use rolling prepaid windows: reserve 30 seconds before creating/continuing a
call (covers OpenAI initialization); renew with at least 10 seconds remaining.
Each window has a stable session/window ID and price snapshot. Authorization,
tariff, credential class and rollout checks precede funding. Reuse allowance ->
grant -> wallet precedence, exact funding splits and existing ledger/Lago dedupe.
Provider sideband setup/control are part of the same voice session, not extra
chargeable proxy requests.

A checkpoint transaction stores the cumulative high-water mark and durable
window settlement intent before advancing the settled watermark. A leased,
fenced reconciler materializes each existing billing usage row/ledger posting
with deterministic IDs; retries reuse them. Settled windows never mutate or
double-post; final usage only adds an unpaid delta and releases unused holds.
Do not count `requests:1` or component byte/token usage on every renewal; a
session-start request component, if configured, happens exactly once. Test the
case where a zero-token primary has a nonzero seconds component.

Failure to reserve stops a call before the funded horizon and closes upstream.
Initial refusal makes **no** provider create call: "Voice couldn't start because
there aren't enough credits" or "Billing is temporarily unavailable. Try again
shortly." Offer billing settings and text chat without claiming text will be
free. Mid-call failure uses a short fixed notice if funded output remains,
otherwise on-screen text. Existing backend work has separate reservations and
does not lose its result because voice funds ran out.

On process/sideband loss, keep known cumulative usage and mark finalization
unconfirmed; reconnect only for bounded observe/close recovery, not task replay.
Never trust a browser duration or invent final provider usage. Settle only the
known/verified window quantities; carry uncertain exposure as an operator
reconciliation item, with the approved 24-hour deadline and platform absorption
of unverifiable remainder before releasing holds. This deliberately accepts
bounded undercollection rather than fabricated charges. No new wallet balance
mutation bypasses the ledger.

Direct OpenAI media means NyxID must verify provider session closure works after
sideband loss, lease loss and credential rotation, and determine the provider's
absolute expiry bound. A crashed creator can leave an unknown provider session
if its POST response was lost. A strict cost ceiling cannot be promised from a
browser heartbeat alone. **Platform-key paid rollout is blocked until forced
closure/orphan behavior and initialization invoices are measured**; record
residual bounded exposure explicitly rather than claiming WebSocket closure
necessarily terminates a separate WebRTC session.

## 13. UI, mobile and accessibility

Stay inside the existing Assistant shell/thread. The same composer shows Send
when text **or attachments** are present, mic when truly empty and eligible,
and a separately reachable Stop when work is running. Pending/failed uploads
retain their send blocking and are not silently attached to speech. Starting
voice on a draft adopts its persisted thread exactly once and preserves the
typed draft if the user switches modes.

Voice mode is a compact panel/sheet in the chat's existing content grid: agent
identity, restrained orb or waveform, current state and live captions; running
tasks with progress/queued/waiting-card state; Mute, End, push-to-talk, voice
choice and a settings popover for provider/model/key source. Keep the 758px
outer grid, 30px identity gutter, 680px content column and measured composer
controls. The full transcript/cards remain available without ending the call.
End voice and Stop task are distinct labelled controls. Results interrupt only
at a natural pause; an important card also remains visually discoverable.

The full panel has separate **You** and **Assistant** caption regions and a live
elapsed timer. Speaker mute controls local playback only; microphone mute is a
separate control and neither ends billing. Minimize keeps the same media session
mounted in a floating bar with timer, status, microphone mute and End; Restore
returns to the full panel while the chat stays usable. A competing tab shows
**End the current call first**. The server's unique per-person live slot remains
the authority; no browser-only lock or automatic takeover.

On closure, append exactly one compact **Call receipt** through the existing
thread message/sequence path with `via: "voice"`. Its deterministic server
snapshot records elapsed duration, requests started/completed/still queued/
cancelled, confirmations decided, and links to available results. The receipt
is a call-end snapshot, not an LLM summary or a claim that queued work completed.
Its durable session-bound identity and transactional marker deduplicate normal
close, disconnect recovery and reconciliation. Results that settle later remain
in the thread through the usual result path.


Follow `DESIGN.md`: Space Grotesk headings, Manrope text, JetBrains Mono timing;
semantic theme tokens, warm purple only for identity/active interaction, neutral
idle cards, `rounded-xl` panels, compact standard controls. The waveform reflects
actual input/output and is not a perpetual decorative animation. Respect
reduced-motion and theme contrast; use labelled status text in addition to color.
No new page-level chrome or unrelated layout redesign.

Use TanStack Query hooks, domain Zod schemas and `useAppForm` for preferences.
Keep media objects/transcripts out of persistent Zustand/localStorage settings.
Remember provider/model/voice/source per person; never remember permission as
granted. If a remembered option loses eligibility, show the reason and require
a new explicit selection. Do not auto-switch keys/providers mid-call. Changing
immutable settings ends/restarts voice with cost and task continuity made clear.

Mobile web requires HTTPS, explicit mic/playback gesture, `playsinline`, safe
areas and enlarged **hit areas** around compact controls. Handle Bluetooth
route changes, incoming calls, audio focus, permission revocation and suspended
AudioContexts. Background/lock may suspend capture/playback; show Paused and
close by the heartbeat/idle deadline, then Resume on tap. No promise of
uninterrupted locked-screen operation. Keep accepted work in the thread.

All controls need accessible names, visible keyboard focus, `aria-pressed` for
Mute/PTT, non-color status, escape/dismiss behavior that does not unexpectedly
cancel tasks, and focus restoration to the invoking control. Coalesce caption
announcements into a polite live region; do not flood screen readers with token
deltas or have their speech fight provider audio. Captions remain available
with sound muted. Offer keyboard PTT and ordinary click confirmation for users
who cannot use voice. Touch entry must not raise the keyboard automatically.

## 14. Latency and capacity budgets

Budgets assume a warm service, usable network and already-granted microphone;
permission-dialog time is excluded. Collect p50/p95 by adapter, region,
browser/device class and network condition using timestamps/counts only.

| Interval | Target / estimate | Instrumentation |
| --- | --- | --- |
| Mic tap -> ready | OpenAI 0.8–2.0 s p50, <=4 s p95; Grok 0.5–1.5 s p50, <=3 s p95 | Client tap, admission, provider connection/config, sideband ready, first media |
| Conversational input -> first useful speech | OpenAI 250–700 ms p50, <=1.2 s p95; Grok 450–1000 ms p50, <=1.8 s p95 | Input speech endpoint, provider first audio, actual output playback |
| Delegation -> accepted/queued receipt | <=150 ms p50, <=500 ms p95 excluding delayed input transcript | Provider event, durable admission, receipt send |
| Request -> spoken acknowledgement | <=800 ms p50, <=1.5 s p95 | Input endpoint, acknowledgement playback; not just append ACK |
| Delegation -> normal backend admission | 50–250 ms p50 idle; <=750 ms p95 | Queue claim, key/authority transaction, turn start; busy time separately |
| Quick service read -> usable answer | 1–4 s p50, 3–8 s p95 estimate | NyxAgent first response/tool/settlement; no fabricated fixed provider SLA |
| Activity -> UI / spoken progress | <=500 ms UI p95; first useful spoken progress around 2 s | Event receipt, UI render, coalesced output start |
| Durable result -> announced | <=1 s p50, <=2 s p95 at next conversational pause | Commit, append, first result audio; queue while user speaks separately |
| Grok relay overhead | 40–150 ms network estimate + 20–60 ms loopback; provider transcript gating adds unverified delay | Compare regional direct timing in lab, relay queue and player timestamps |
| Candidate barge-in -> duck / verified interrupt | <=100 ms duck, <=500 ms verified interruption target | Candidate speech, real input confirmation, playback stop |
| Explicit Stop -> cancellation requested | <=500 ms p95 server request; existing worker polls at 250 ms | Durable Stop, upstream drop, terminal settlement separately |

At 24 kHz PCM16 mono, each uncompressed direction is 48 KB/s; bidirectional
client relay payload is about 96 KB/s plus protocol overhead (base64 adds ~33%
on a JSON audio hop). Count both upstream/downstream server legs for capacity.
Set global/per-replica admission limits from load tests, independent of text-turn
permits. A voice call does not hold a backend turn permit while merely chatting.
Backpressure and long result narration must not delay receipt of Stop/mute.

## 15. Failures and observability

| Failure | Required behavior |
| --- | --- |
| Mic denied, missing device, unsupported playback | No provider creation/charge before usable capture and playback setup. Explain browser permission action; keep text usable. Stop already-created sessions on late failure. |
| Provider rejects model/key/capacity | Fixed actionable message; release unused holds, preserve known initialization charges; no credential dump or silent provider fallback. Mini unavailable is explicit. |
| OpenAI sideband lost while audio lives | Immediately stop delegation admission and tell UI to pause media; attempt bounded authenticated reattach solely to observe/close. Gap means transcript/usage uncertain. Rebuild a new session with saved text only after safe closure/user action. No client-forwarded events become authority. |
| Grok upstream/client relay disconnect | Close upstream on client heartbeat expiry, checkpoint duration, discard raw buffers; retain requests. Reconnect never resends buffered old speech/tool calls. |
| Control WS lag/resync or change-stream outage | Bounded buffers; task snapshot/history refresh and existing poll backstop. Refuse work when authoritative persistence/ACL state cannot be established. |
| Replica crash/expired lease | Fenced recovery closes orphan session where possible, settles known usage and releases slot only after bounded recovery. Reconcile admitted request against its turn ID; never replay a started/lost/unknown turn. |
| Billing denied before or during call | No start without initial funds; close before next unfunded window. Clear UI message, exact ledger recovery, keep accepted task results. |
| Backend error or insufficient credits | Persist normal failed reply; announce the safe existing error. No credential replacement/retry for insufficient credits. An uncertain action stays uncertain. |
| Queue full, rate limit, turn busy | Busy request visibly queues within bounds; full/rate-limited request is refused with retry guidance, not silently dropped or labelled started. |
| Org membership/grant/scope revoked mid-call | Live recheck rejects next action/delivery, Stop affected work, close provider audio and clear pending secret challenges; no personal fallback. Pre-admitted effects may already have completed. |
| Thread key rotation/revocation, agent destroyed, account logout | Stop new delegation, expire challenges; normal key/binding recovery rules. End session on identity loss; no cross-actor late events in UI. |
| Duplicate provider call/result or delayed old event | Deduplicate persistent request/window IDs; ignore old generation/version, no double work, charge, continuation or notification. |
| False speech / echo | Duck then recover, no backend Stop from VAD; require explicit real input. Offer click/PTT/headphones. |
| Voice ends with task/card pending | Accepted work and card stay in the thread; normal expiry applies. Result notification uses a deduped receipt and user's preferences. Never auto-confirm or silently re-open mic. |

Audits go through the existing append service/hash chain and contain only actor,
agent/thread/local session/request IDs, provider/model enum, state, counts,
duration provenance and safe reason codes. No keys/key identifiers, ciphertext,
SDP/ICE/IP candidates, client secrets, transcripts, skill bodies, tool payloads,
raw audio or provider error text in logs/audit/traces. Scrub request
bodies, WS frames and `Authorization` at application/ingress/error-reporting
layers; disable assistant wire capture for voice. Debug is redacted. Metrics
have bounded labels; avoid user/session IDs as time-series labels.

Persist transcripts only in the authorized thread, not in the voice-session
record or logs. No raw audio on disk, object storage, database, analytics or
diagnostic traces; transient transport buffers are explicitly bounded and
discarded. Set OpenAI `store:false`, avoid xAI resumption and disclose that
providers still process audio under their own data policies. NyxID cannot claim
its no-recording rule changes a provider's default retention policy.

## 16. Test and acceptance plan

### Deterministic/unit and integration coverage

* Provider fixtures: exact Live start/attach/client permissions, late/reordered
  transcript deltas, delegation without text, 500-token bounds, append errors,
  cumulative/final usage, close without finalization. Grok cumulative transcript
  correction, final events, multi-function batch, force-message lifecycle,
  manual commit/cancel/truncate, aliases and unknown events. No paid calls in CI.
* Replica-set integration: two replicas starting the same person/thread; duplicate
  request IDs; busy turn FIFO; atomic queue -> message -> turn mapping; transcript
  writes racing settlement; Stop vs claim/result/approval; lease expiry; stale
  worker output; crash before/after provider creation and before/after commit.
  A started turn is never re-executed on recovery.
* Authority matrix: personal NyxBot, granted/denied specialist, B1 allowed/denied
  operation and WS-scoped service, B2 read/revocation, B3a Admin/Member/Viewer,
  removed member/inactive org, actor-private histories, org-resource restrictions,
  first-party cookie/JWT, OAuth client, API key, relay, delegated account-read,
  guest and malicious provider/browser function frames. Verify live revocation.
* Credential tests: live shared ACL before any decryption; platform vs personal
  and org own key; disabled/revoked bindings; retained own key; gateway/node refusal;
  no arbitrary destination/header selection; same actor and billing resolver.
* Financial tests: 12 -> 15 cumulative seconds charges 15 total, 90-second OpenAI
  session never 105; confirmed initialization-only charge, zero/short session,
  disconnected finalization, duplicate checkpoints, fixed-price windows, exact
  allowance/grant/wallet splits, token-primary-zero/duration-component-positive,
  missing BYOK lane or duration component/metering-only with no quote, pending primary/component, renewal failure, replayed
  reconcile, ledger integrity and Lago dedupe. Unconfirmed seconds cannot become
  invented final charges. Test Grok subsecond carry without per-window rounding.
* Confirmation tests: approve, deny, unclear twice, silence and expiry; overlapping
  or echoed “yes” ignored; model claims and injected tool output claiming user
  approval ignored. Click vs voice yields one decision/continuation. Loss of org
  access before decision refuses. Current-key/action-digest changes and Stop races
  preserve one-use authority. Test output completion plus playback watermark,
  sealed input provenance, post-inference revalidation and browser suppression.
* UI tests: empty mic vs text/attachment Send, visible Stop during queued work,
  draft adoption, provider/source loss, live captions and task rows, muted billing
  indication, keyboard/focus/reduced-motion/contrast, actor change and late events,
  server vs browser continuation dedupe, upload isolation and mobile layout.

### Repeatable acoustic/browser QA

Run on current and previous stable Chrome and Firefox (macOS/Windows/Linux),
Safari on macOS and real iOS Safari, plus Android Chrome. Record exact versions,
device/audio route, room, distance, output volume and settings; do not equate
headless Chromium with physical AEC.

1. Use the same neutral phrase corpus at 25/50/75% speaker volume, laptop mic at
   0.5 m and 1 m; quiet room, fan/typing and background speech conditions. Run
   ten 30-second assistant-only segments per setting. Target **zero** backend
   requests/Stops/confirmations and <=1 false output interruption per ten minutes.
2. Speak 20 meaningful interruptions while output plays, plus 20 "mm-hmm"/
   cough/backchannel samples. Target >=95% meaningful interruptions recognized,
   <=500 ms p95 verified output interruption and no task cancel from backchannels.
3. Repeat with internal speakers, wired headphones, Bluetooth and output-device
   switches. Verify capture constraints, loopback-only playback (no doubled audio)
   and echo self-test recovery. A failing browser stays PTT/headphone-labelled.
4. Start long calendar/machine/specialist jobs in a test account; ask two more
   questions, change a detail, Stop one, approve one by speech, then end
   audio. Verify queue order, progress, exact authority, one settled result and
   one notification, without requiring another user question.
5. Repeat disconnects during startup, speech, sideband append, billing renewal,
   card arming and settlement; remove org membership mid-call. Exercise reload,
   two tabs, logout/login as another person, mobile lock/incoming call/autoplay
   rejection, 150/300 ms RTT, 1/5% loss and constrained bandwidth.

Automate PCM fixtures containing clean user speech, assistant echo delayed
40–250 ms, reverberation, noise, double talk, silence and clipped speech. Browser
fake microphone tests cover state transitions, audio worklets, resampling,
bounded buffers and no-hard-cancel policy; play/record timestamp probes measure
the loopback latency. A physical speaker-to-mic harness validates the acoustic
path. Use synthetic/generated test audio only; no customer audio fixtures.
Transcription accuracy and AEC metrics require measured reports, not mocks of success.

Before paid enablement, run a small explicitly budgeted provider contract/billing
trial covering entitlement, restrictive OpenAI client permissions, orphan/forced
closure, <15-second calls, Grok manual transcription during output, exact text-input
charges and invoice-duration reconciliation. This design phase does not run it.
Do not broadly enable voice until the automatic laptop-speaker experience passes;
PTT is a usable fallback and an honestly labelled earlier beta.

## 17. Shippable implementation phases and rollout

Phases 1–3 are approved. Phase 4 is authorized on `feat/voice-grok`, stacked on
Phase 3 with its HMAC credential-fingerprint fix. Each implementation phase is its own
PR to main; the owner reviews, versions and merges. Do not commit during this
implementation session. Composer UI is owned by another implementer; Phase 2
must not edit either composer. U1 (`36e3ef04`, 0.48.0) is now merged into the
Phase 3 worktree; its one-shot helper and composer are the integration base.

| Phase | Deliverable that can ship independently | Exit gate |
| --- | --- | --- |
| 2. Shared foundations | Backward-compatible optional fields; guarded routes; voice options/preferences; common request/turn bridge, transcript identity/acknowledgement fix and server continuation receipts; duration metric support and reconciliation. Flag stays off; text regression suite passes. | All consumers can read new fields/metric, money/authority/queue/Stop concurrency tests pass; provider uncertainties remain beta rollout gates, not Phase 2 dependencies. |
| 3. OpenAI private beta | `gpt-live-1` WebRTC + restricted sideband, same-thread delegation, progress/results, queue/Stop, captions, source selection, duration funding, end-and-notify behavior. Spoken read-back with playback/echo gates, stateless classification and server-owned continuation; cards remain optional fallback. | End-to-end parity with NyxBot and personal/org specialists, forced-close and billing gates, desktop/mobile AEC QA. Enable selected people only. |
| 4. Grok private beta | Constrained relay, async function receipts and output arbiter, real reported-token provenance, duration tariff, loopback AEC and manual interruption gate. PTT/headphone mode can ship first with clear labelling if automatic mode is still gated. | Same authority/billing/task tests, manual-input contract and acoustic matrix; no unsupported VAD configuration claims. |
| 5. Wider rollout and optimization | Canary cohorts then opt-in general availability for validated browser/provider combinations, measured regional capacity and latency tuning. Add Mini only after official support/entitlement verification. | Error/cost/echo/latency targets hold, no ledger drift or duplicate actions, rollback exercise passes. Vetted service fast paths require their own evidence/review. |

Upgrade **all** auth/proxy/MCP/assistant/worker/billing replicas and the frontend,
CLI/admin schemas before writing `voice_seconds` prices/allowances or enabling
`assistant:voice`. Older enum readers cannot deserialize the new billing metric;
old acknowledgement clients may double-start browser continuations, and old
workers do not know transcript-only rows. Verify the existing exact-billing
migration is complete; this feature does not repeat or bypass its cutover.
Create additive indexes before enabling; preserve legacy field defaults and
all unrelated traffic paths without new voice database reads.

Enable a test-person override, then a small cohort, with platform pricing and
any paid own-key pricing explicitly configured.
Phase 3 uses a separate default-off `assistant:voice-openai-platform` gate for
platform-paid sessions. Platform sessions require synced duration pricing.
BYOK allows disclosed metering-only use when its lane is absent or its synced
primary has no duration price; explicitly authored primary/duration prices must
be synced.
Unknown closure retains the person's live slot until provider finalization or the call deadline. Leases
renew during setup and slow inference; settlement retries retain their own
completion marker. A closed socket is never proof of zero provider cost.
 Track known/unknown duration, setup failures,
lost sidebands, duplicate suppression, queue delay, interruptions, confirmation
failures and exact settlement lag. Flag checks apply at admission; the active
session sweep treats flag disable as orderly voice shutdown. Queued accepted
backend work remains accessible/controllable in text.

Rollback order: disable global/cohort/person enabling overrides; stop accepting
sessions and drain/close existing audio; settle/reconcile all usage;
finish or explicitly Stop queued backend work; verify no active voice leases or
server-owned pending continuations. Keep new readers/enforcement/reconcilers
running while voice rows or `voice_seconds` configuration exist. Do not restart
pre-metric binaries against those records. A rollback to older binaries requires
a separately reviewed data/config migration; flag-off alone is not sufficient.
Existing platform ACL, B1 scopes and B3a enforcement remain active throughout.

## 18. Approved decisions and remaining verification gates

The owner approved GPT-Live 1 only at launch and subsequently required spoken
read-back confirmation (§11), replacing the earlier on-screen-only decision.
Server-owned continuations remain; private-code routes/collection and local ASR
are removed. Cards remain transcript records and optional fallback. Unverifiable OpenAI
initialization or orphan cost is platform exposure, never an invented user debit;
forced close and bounded reconciliation remain a paid-platform rollout gate.
Grok launches with PTT/headphones labels; automatic mode waits for successful
manual transcription during output and loopback AEC tests.

Per-provider seconds tariffs use admin lanes and existing reservation, settlement
and ledger primitives (allowance → grant → wallet, deterministic IDs). Grok bills
completed seconds, waiving the terminal fraction; confirmed text-input cost is
folded into the disclosed tariff. Uncertain finalization expires after 24 hours.
There are no source-code dollar constants. Limits are 30 minutes per call,
three minutes user idle with a 15-second warning, one transmitting call per
person and ten queued requests per thread. Completion notifications are
metadata-only and preference-based. Use `store:false`, no NyxID recording and
visible data-policy disclosure.

Remaining provider verification gates (not requests to reopen approved policy):

- Official Mini availability and account access before displaying it.
- OpenAI forced close, orphan recovery and failed/short initialization invoices.
- xAI text-input charge categories and observed duration boundaries.
- Grok manual transcription during output and browser loopback AEC quality.
- Deployment/account provider retention terms before enabling paid voice.

## Sources

Official sources retrieved 2026-10-03. Versioned behavior and prices must be
rechecked during implementation; this document records the observed contract.

[oa-models]: https://developers.openai.com/api/docs/models
[oa-model]: https://developers.openai.com/api/docs/models/gpt-live-1
[oa-webrtc]: https://developers.openai.com/api/docs/guides/voice-webrtc?api=live
[oa-controls]: https://developers.openai.com/api/docs/guides/voice-server-controls?api=live
[oa-sideband]: https://developers.openai.com/api/reference/resources/live/sideband-websocket
[oa-delegation]: https://developers.openai.com/api/docs/guides/live-delegation
[oa-sessions]: https://developers.openai.com/api/docs/guides/live-conversations
[oa-cost]: https://developers.openai.com/api/docs/guides/voice-latency-cost?api=live
[oa-prompt]: https://developers.openai.com/api/docs/guides/live-prompting
[xai-voice]: https://docs.x.ai/developers/model-capabilities/audio/speech-to-speech
[xai-ref]: https://docs.x.ai/developers/rest-api-reference/inference/voice
[xai-schema]: https://docs.x.ai/voice-realtime.ws.json
[xai-tokens]: https://docs.x.ai/developers/model-capabilities/audio/ephemeral-tokens
[xai-pricing]: https://docs.x.ai/developers/models
[aec-spec]: https://www.w3.org/TR/mediacapture-streams/#dom-echocancellationmodeenum
[aec-mdn]: https://developer.mozilla.org/en-US/docs/Web/API/MediaTrackConstraints/echoCancellation
[aec-chrome]: https://developer.chrome.com/blog/more-native-echo-cancellation
[aec-webkit]: https://webkit.org/blog/7726/announcing-webrtc-and-media-capture/
[engine]: 08-nyxagent-engine.md
[nyxbot]: 09-nyxbot-orchestrator.md
[uploads]: 10-uploads.md
[org]: ../ORG_AGENTS.md
[b1]: ../AGENT_OPERATION_SCOPES.md
[b2]: ../AGENT_SKILLS.md
[platform]: ../PLATFORM_KEYS_AND_INFERENCE.md
[exact]: ../BILLING_EXACT_ACCOUNTING.md

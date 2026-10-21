# Assistant Chat Frontend Contract

Last verified against Aevatar console commit `e7ba2e6eb` (2026-08-25).

The assistant page presents one continuous transcript, an anchored composer, and a conversation sidebar. Network and stream states change the same surface in place; loading, thinking, streaming, terminal, and recovery states do not replace the application shell.

`frontend/src/pages/assistant.tsx` selects the HTTP-fixture, typed actor, or
Direct surface. Typed query and turn ownership live in
`frontend/src/hooks/use-assistant-chat.ts`; Direct's memory-only seam lives in
`frontend/src/hooks/use-assistant-direct.ts`. Visible transcript contracts live
in `chat-message.tsx`, `chat-composer.tsx`, `assistant-sidebar.tsx`, and the
card components.

## Conversation selection

The route may select a durable conversation or the entry screen. The current
conversation is the history query result's canonical ID when available,
otherwise the selected route ID. A typed conversation becomes addressable only
after a valid `RUN_STARTED` adopts its server-owned `nyxid-chat-*` identity.

Selecting a sidebar row changes the active conversation and requests composer
focus. Selecting New chat navigates to the empty chat screen and requests focus.
New chat issues no create request. A first send posts typed `text` lazily; it
does not allocate a workflow placeholder or invoke a second engine.

A selected `chatc-*` row is historical. Its transcript can be read and deleted,
but its composer and actor controls are disabled. It cannot be continued,
stopped, steered, approved, or used as an action-continuation target.

A stale or missing conversation returns to a usable new-chat state. It does not retain a composer bound to a nonexistent durable ID.

Each typed conversation owns a separate local session and AbortController.
Switching away never redirects a stream into the newly selected chat. Switching
back while it is running restores its live loading state; switching back after
it settles shows the completed local turn. A draft ID is aliased to the
canonical `nyxid-chat-*` ID at `RUN_STARTED`, and a queued alias navigation may
not override a newer reader selection.

With `experimental:direct-chat-engine` enabled, a draft or `direct-*` route uses
the in-memory Direct seam. Explicit `nyxid-chat-*` and `chatc-*` routes still use
the canonical actor/history page. Direct conversations are not persisted.

## Thread states

The thread distinguishes execution state from transcript content. An active turn can have no printable output yet, printable streaming output, or structured activity without text.

### Idle empty

When no turn has run and there are no messages, the thread shows:

```text
Ask NyxID to help with services, access, and account operations.
```

This empty state must not appear after a turn has started or ended merely because projection is delayed.

### Thinking

Thinking begins as soon as a send or continuation is pending and there is not yet a printable assistant tail. The thread renders one assistant identity row with a loading halo and dots where the answer will appear.

The semantic contract is:

```html
<article role="status" aria-label="Assistant is thinking">
  <span data-assistant-halo aria-hidden="true">...</span>
  ...
</article>
```

`[data-assistant-halo]` is the stable test marker. It exists only while the
streaming assistant identity has no printable content. Settled assistant
avatars never carry it. The halo itself is decorative; the containing status
announces the state.

### Streaming before content

When the assistant message exists but its text block is still empty, the answer slot displays three streaming dots. In this placement the dots own the live-region semantics:

```html
<span
  data-streaming-dots
  role="status"
  aria-label="Assistant is answering"
>...</span>
```

`[data-streaming-dots]` is the stable test marker. The dots remain while the
canonical assistant text is empty, including when reasoning, activity, or a
card has already arrived, and disappear once text arrives or the message
settles.

### Streaming content

After a text delta or another printable assistant block appears, the thread renders the actual block. Text updates in place with its streaming treatment. The thread follows the tail while the reader remains within 48 pixels of it. A deliberate upward scroll suspends automatic following; streaming must not pull the reader back down.

A newly sent user message always restores tail following so the optimistic echo is visible.

### Settled

Completed, blocked, failed, and stopped turns stop thinking and streaming
treatment. Open text and activity are finalized according to the terminal. A
blocked turn keeps its recovery card visible. A stopped turn reflects local
Stop or server `RUN_STOPPED` and does not show a red error or an empty-turn
marker solely because no assistant text arrived.

### Empty-terminal detection

If a non-stopped turn closes without printable content, the UI waits 700
milliseconds before rendering a detection-only marker:

```html
<span data-empty-turn-error class="sr-only" aria-hidden="true"></span>
```

The marker is observable by tests and diagnostics but is not a visible alert and is hidden from the accessibility tree. Production evidence shows that a content-free terminal can be legitimate, and the console does not present a "didn't reply" alert. The grace period protects against status and transcript events arriving in opposite orders. A fresh episode resets the timer in render state, so an old settled flag cannot leak into a new turn.

The current local session is authoritative for detection. A later route restore
uses the stored assistant tail as evidence; no terminal-time transcript reread
is required.

## Stable semantic markers

The browser suite intentionally asserts three markers rather than CSS implementation classes:

| Marker | Meaning |
| --- | --- |
| `[data-assistant-halo]` | active thinking identity treatment, decorative inside a named status |
| `[data-streaming-dots]` | answer pending at the future content position |
| `[data-empty-turn-error]` | hidden detection marker for a closed non-stopped turn with no printable content |

These attributes are part of the testing contract. They can move with equivalent markup but must not be removed without updating both component tests and Playwright helpers.

## Message composition

Each canonical `ChatMessage` renders one identity row. User content is a compact
bubble. An assistant row composes, in order:

- a collapsible thinking/reasoning disclosure;
- a collapsible step and tool-call activity disclosure;
- smoothly revealed Markdown text;
- typed authorization recovery as `ConnectCard`;
- `MEDIA_CONTENT` output as `ArtifactBlock`; and
- an error notice when the message settles in error.

The actor projection renders the task plan, pending input, approval, and v4
action cards beneath the message list. There is one actionable approval
surface: `nyxid.approval.request`. The shipped `ApprovalCard` remains visible
after resolution using `latestApprovalResolution`; while a 202 acknowledgement
is waiting for the committed fact it shows `decision_submission`.

Authorization recovery accepts the typed `nyxid.authorization.required` fact
and a strict readiness DTO carried by `TOOL_CALL_END`. A successful connection
settles the card. Arbitrary workflow intervention and AG-UI tool-approval
surfaces are diagnostic accumulator data only and are not interactive.

Media artifacts have an 8,000,000-character inline cap. Current history rows do
not carry media, so an artifact is visible in the live local message but is not
reconstructed after reload.

## Long transcripts

Rows re-render only when their message or the page's projection version changes,
so a streamed token re-renders the streaming row alone. Transcripts longer than
30 messages are windowed: only rows near the viewport are mounted, measured at
their real heights and the rendered row gap (display spacing scales it). Windowing
starts when a thread opens long or while the reader follows the newest message,
never under a reader who scrolled up, and then stays on for that thread. The
thread still opens at its newest message, follows a
streaming reply, keeps the reader's place when older messages load above, and
brings an unmounted row into view for a linked review card or a
`#message-<id>` link. Shorter transcripts stay whole so find-in-page works.

## Optimistic user messages

The composer clears before `onSend` resolves. The page therefore carries a pending send echo from the start of the mutation until the transport projection contains the corresponding new user message.

The echo identity is count-aware. An older identical user message does not suppress the pending one; the number of matching messages must advance beyond the send-time snapshot. This keeps repeated prompts visible without duplicating the current prompt after projection lands.

The optimistic echo covers both:

- first send, while the local conversation and query identity are being created; and
- existing-conversation send, during the shorter gap before the transport emits the user message.

An HTTP failure before the stream is accepted restores the submitted message
and removes the optimistic turn. A 30-second start timeout or any post-start
failure settles the existing thread with an assistant error and resolves the
send; the composer is freed without restoring duplicate text.

## Composer

The composer is anchored at the bottom of the chat surface. The thread reserves its measured height up to half the visible viewport or 480px, whichever is smaller. Beyond that threshold, manual expansion overlays the chat without increasing its bottom inset. The opaque composer surface and fade move with the top edge.

NyxAgent uses one settings menu in the composer footer. By default, it contains
only **Thinking** (a toggle) and **Permissions**; the trigger shows the effective
access level, adding **, Thinking** only when enabled (for example, **Full access,
Thinking**), and exposes no model names. Thinking on selects `nyxagent/research`,
and off selects `nyxagent/chat`, using the same draft selection and persisted
thread-model mutation as the advanced selector. The toggle is disabled while a
turn runs or its target profile is unavailable.

The `assistant:model-controls` feature flag defaults off. Enable it through the
existing platform-admin user or cohort overrides for internal users. Flagged
users see **Reasoning**, **Model**, and **Permissions**, without the Thinking
toggle, and their trigger shows the current model name. The agent-details model selector uses the same flag. This is a UI
rollout flag; it does not change the model-selection API's authorization.

Model and permission choices open in submenus on desktop and
expand inline on narrow screens. Permissions are listed as **Full access**
(`autopilot`), **Auto** (`nyxmode`), and **Ask every time** (`ask_every_time`), without
explanatory paragraphs. An agent override remains authoritative and makes the
permission choices read-only; workspace changes cannot silently override it.
Permission changes are also disabled while a turn runs. Models use
upstream display names when present, with **Default model** for the legacy
`nyxagent/chat` alias. The routing ID remains unchanged. Model changes are disabled
while a turn runs. New-chat selections are local to that composer, passed on the
first send, and applied to any persisted upload draft before submission.

Connection selection is separate from the permission/model menu. **Agent Details →
Connections** groups accounts by service and identifies personal and organization
ownership. NyxBot's Details lets the owner edit the existing saved connection
order; this preference is account-wide and applies across agents. Eligible
personal connections still precede organization connections. Specialist Details
shows only granted connections; its Access section owns service grants and
operation/resource limits. Choosing a connection does not impose folder/database
restrictions or widen access.

For a one-message exception, the composer's **Add context** (+) menu offers
**Attach files** and **Use a connection**. The connection dialog is searchable
by service, account, slug or organization and groups results by service. Selecting
an account displays a removable chip naming the service, account, owner and
**This message**, visible before sending. It clears after successful send and
on navigation; a failed send retains it. If the selected connection becomes
unavailable, sending is refused until the owner removes or replaces it. Voice
is unavailable while a text-message connection is selected. Submission retains
the leading `/service <UUID>` and actual newline; the server binds the choice
to the admitted turn. Organization connections still require the existing action
card in Auto and Ask every time.

The composer stays in one row while empty or while its text fits alongside the
controls. It expands only when text wraps or contains a newline, then collapses
again when the text fits. In the expanded composer, the text occupies the full
upper row. Attachments sit
at the bottom left; the single settings trigger, optional voice action, and
Send/Stop sit at the bottom right. When enabled by `assistant:voice`, the voice
waveform remains available for an empty idle draft. Send is absent for empty or
whitespace-only drafts, regardless of voice availability. Non-whitespace text or
attachments replace Voice with the send arrow in the same action slot; pending
uploads keep Send disabled. Clearing the draft restores Voice. An active turn
keeps Stop available, and an active voice call retains its in-composer call controls.
The menu is bounded by the viewport and scrolls long model lists.

For flagged users, the Reasoning row is visible but marked **Unavailable** and
is not selectable: the checked-in NyxAgent turn contract has no
supported effort field. Do not infer supported effort levels from model names.

### Input rules

- The textbox accepts at most 32,768 characters.
- Submission trims the whole value.
- Blank or whitespace-only input cannot send.
- Enter sends.
- Shift+Enter inserts a newline.
- Enter during IME composition does not send; both composition state and key code 229 are guarded.
- The composer uses a minimal Tiptap/ProseMirror editor with atomic inline mention pills. Agent pills show display names; attachment pills show numbered handles: `image-1`, `file-1`, and `text-1` for long pasted text. Autocomplete, plain-text paste, copy, undo/redo, and Shift+Enter are supported. Serialization preserves the existing `@handle` text contract; attachment handles still resolve before sending. Draft storage remains plain text. Undo history resets across conversation/owner changes, after sending, and when removing an upload so deleted references cannot return. Formatting controls are not enabled in this first iteration.
- Composer attachments occupy one horizontally scrollable row: square image previews and compact document cards with filename, type, and size. Long pastes show a text excerpt. Corner controls remove individual files; upload progress and errors remain within the fixed card height. Attachment mention handles remain available through autocomplete and card hover text. Adding attachments scrolls the row to the newest card; upload progress and removal preserve manual scrolling. Edge fades indicate additional cards offscreen.
- The text editor grows from one through four rows by default, then scrolls internally. Dragging the expanded composer’s top-edge handle changes its visible height even for short multiline drafts. The complete surface, including controls, attachments and padding, can reach 90% of the visible viewport, bounded below the chat header. Chat space stops growing at half the viewport or 480px; further expansion overlays the transcript. Shrinking below that threshold restores measured spacing. Short single-line drafts still collapse; the chosen height is retained within the mounted composer. Arrow keys adjust it, Home/End select its limits, and double-click resets it. Window and software-keyboard viewport changes recalculate the limit, keeping the grip and controls reachable. A persistent, draggable thumb appears while text overflows. Expanded controls sit 2px below the editor.
- A long one-line draft switches controls to the multiline layout before text overlaps the button.

The idle placeholder is `Message NyxID Assistant...`. During an active turn it is `Assistant is working...` and the textbox is disabled.

### Send and Stop

When idle, the control is an icon button named `Send message`. It is disabled for blank content or while the send mutation is still starting. Once the turn is active, it is replaced in the same control slot by an icon button named `Stop assistant turn`.

Stop always aborts the selected local reader, including while response headers
are pending and state version is zero. When authoritative actor/turn identity
and a positive state version are available, the client also sends a best-effort
typed `task.stop`. Version fencing may disable actor controls but never local
Stop.

The controls have stable dimensions, so the send-to-stop transition does not resize the composer.

### Draft ownership

Drafts are owned by the authenticated user and keyed by screen or conversation:

```text
screen:{entryScreen}
conv:{conversationId}
```

Writes are debounced by 300 milliseconds and flushed on unload and unmount. Switching conversations saves the outgoing live draft and loads the incoming one. The first durable conversation adoption migrates an otherwise empty screen draft into the conversation key. A draft from another authenticated user is never shown.

Sidebar rows show an inactive conversation's normalized draft preview, limited to 80 characters, with screen-reader prefix `Draft:`. The active row does not repeat the composer draft below its title.

### Focus

Desktop/fine-pointer entry focuses the composer. Coarse-pointer devices avoid autofocus so opening a chat to read does not raise the software keyboard.

Explicit sidebar and New chat selections request focus. Canonical-ID repair and browser navigation can restore focus only when focus is parked. At turn end, focus returns only if the composer held it before disabling and the reader has not interacted elsewhere by pointer, keyboard, wheel, or touch. A request that occurs while disabled is held until the field is enabled, unless the reader moves on.

Restored drafts place the caret at the end.

Implementation: `frontend/src/components/assistant/chat-composer.tsx` and `frontend/src/stores/assistant-draft-store.ts`.

## Setup links in chat

NyxBot thread and group transcripts, and the retained actor transcript, wrap
their existing message components in `AssistantLinkModalHost`. An ordinary
click on a same-origin `/connect/nyx_clk_<token>` or
`/channel-bots/connect/<platform>` link opens the shared setup component in
place. Links remain ordinary URLs: direct navigation uses the standalone page,
modified clicks keep browser behavior, and unrelated or external URLs are not
intercepted. Channel links retain only `label` and `target_org_id` query values.

The chat adapter owns link selection, the overlay, and focus restoration to the
initiating link. The connector overlay displays `ConnectLinkContent` directly
with a close button; its standalone page renders the same content. Channel
setup reuses `ChannelBotSetup` with an opt-in `stayInPlace` presentation, while
existing callers retain their navigation defaults. Completion stays in chat.
These are frontend presentation choices; no backend or message protocol is
changed.

External OAuth authorization still uses the provider's window. It is reserved
during the submit gesture, and receives its recovery token in its own session
storage before navigating away. The return page can therefore complete even
when provider isolation removes `window.opener`; the chat also polls link
status. Dismissing setup while its request is pending cannot open a later
orphan window. Device-code and token entry keep their existing setup behavior.

## Sidebar and history

The sidebar has a primary New chat command, workspace destinations, and a Chats navigation list. The chat list is a semantic `nav`. Each conversation title is a button whose accessible name is the title.

Conversation metadata comes from the fully drained NyxID index. The row displays the server title and preserves newest-first ordering from the backend. The active row remains selected across placeholder alias adoption by using the canonical history ID when available.

Each row has a separate options button named:

```text
Options for {conversation title}
```

The button opens a menu; opening the menu does not itself open a delete prompt. On pointer-less devices the options button remains visible and tappable. On hover-capable devices it appears on hover, focus, or while open.

The mobile sidebar is a drawer with controls named `Open chats` and `Close chats`. The application brand link is named `Assistant home`.

## Delete UX

The row menu's Delete command opens a dialog named by its title:

```text
Delete chat?
```

The description names the conversation and states that its history is removed permanently. Cancel dismisses without mutation. Delete may show pending state.

One shared dialog serves the list. Pending delete identities are stored as a set, allowing a request to outlive a dismissed dialog and preventing duplicate submission for the same conversation even if another conversation is deleted concurrently. A failed request leaves the dialog open and retryable. A successful request closes the matching dialog and removes the row through query updates.

Deletion is refused while that conversation is streaming. A successful delete
removes its local session and server history row and navigates an active reader
to exactly one fresh draft.

Implementation: `frontend/src/components/assistant/assistant-sidebar.tsx`,
`frontend/src/hooks/use-assistant-chat.ts`, and
`frontend/src/lib/assistant/chat-history-api.ts`.

## Loading and fetch failures

All toasts on `/assistant` routes appear at the top right on desktop and top
center on mobile. NyxBot chat send, stop, retry, and group-action error toasts
show a short explanation and next step, without diagnostic IDs. The inline
chat notice retains the error explanation and trace ID after the toast disappears;
a new send or control attempt clears the previous request notice. Request
notices stay scoped to their conversation, including new-thread adoption.
Conversation/group load failures and the latest failed turn appear above the
chat, outside its scrolling transcript. Historic failures retain their message
context and retry controls. Inline HTTP failures use short, actionable copy and show
the server-issued `X-NyxID-Trace-Id` when available; internal and upstream 5xx
details are never displayed. The server generates a fresh UUID for each
assistant HTTP request. The ID is searchable as `trace_id` in the
`assistant_request` server span, including the detached NyxAgent turn. Persisted
turn failures use the durable turn ID (searchable as `turn_id` in server logs and
stored messages). Approval `request_id` values are unrelated and never used as
support trace IDs.

A broken accepted reply stream retains the HTTP trace and tells the reader
NyxBot may still be working. It never restores the already-admitted prompt or
uploads. If no response arrives, the UI explains that the message may have
arrived and has no server trace ID to display. IDs are diagnostic references,
not credentials or permission to access a conversation.

When a selected conversation is loading and no messages are available, the chat surface shows:

```text
Loading conversation...
```

It does not briefly show the new-conversation empty state.

A transcript 404 for an ID still present in the drained index shows a
no-transcript-yet notice above a usable composer. A 404 for an ID absent from
the index repairs the route to New chat. An empty `projectionStatus: pending`
transcript is reread on the bounded 250/500/1000/2000 ms cadence and remains a
nonblocking placeholder if exhausted. Other transcript failures appear as a
status notice above the usable thread and composer.

Conversation-list failures appear as a sidebar notice. HTTP and session state
remain scoped to the current authenticated identity; attributed dead-session
401 codes clear auth, while an uncoded upstream 401 preserves it.

## Accessibility contract

The surface exposes these stable roles and names:

| Element/state | Role or accessible name |
| --- | --- |
| thinking row | `role=status`, `Assistant is thinking` |
| pre-content answer dots | `role=status`, `Assistant is answering` |
| empty terminal detection | hidden `[data-empty-turn-error]`, no live role |
| transcript fetch failure | `role=status` |
| composer | textbox |
| send button | `Send message` |
| stop button | `Stop assistant turn` |
| chats list | `navigation`/`nav` |
| conversation row | button named by conversation title |
| row menu trigger | `Options for {title}` |
| delete confirmation | dialog with title `Delete chat?` |
| artifact download | `Download {artifact name}` |

Decorative icons, halo sprites, and dot children are hidden from assistive technology. Live roles are removed or hidden during exit transitions. Action and approval components expose their own controls through native buttons and dialog semantics.

## Rendering continuity

The page must not render incoherent overlaps or empty gaps during state handoff. In particular:

- the optimistic user echo covers conversation allocation;
- thinking begins before a transcript query identity exists;
- dots occupy the future answer slot;
- loaders disappear without retaining stale active markers;
- empty-terminal detection waits its 700 ms grace;
- loading detail does not show the empty-chat prompt;
- active streams preserve local transcripts while list metadata refreshes;
- scroll following respects deliberate reader position; and
- fixed message, card, toolbar, and composer dimensions prevent dynamic content from shifting controls.

The Playwright continuity probe described in [Testing and gaps](07-testing-and-gaps.md) observes DOM mutations to catch transient states that ordinary final assertions would miss.

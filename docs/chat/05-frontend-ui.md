# Assistant Chat Frontend Contract

The Aevatar chat contract is pinned in `tests/fixtures/assistant/aevatar-chat-contract-pin.json`. See the pin section of [README.md](README.md).

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

The composer is anchored at the bottom of the chat surface. The thread reserves its measured height and fades content behind it rather than clipping messages against a hard boundary.

### Input rules

- The textbox accepts at most 32,768 characters.
- Submission trims the whole value.
- Blank or whitespace-only input cannot send.
- Enter sends.
- Shift+Enter inserts a newline.
- Enter during IME composition does not send; both composition state and key code 229 are guarded.
- The textarea grows from one through four rows, then scrolls internally.
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

# Assistant Chat Frontend Contract

Last verified against `fcb79b18` (2026-08-01).

The assistant page presents one continuous transcript, an anchored composer, and a conversation sidebar. Network and stream states change the same surface in place; loading, thinking, streaming, terminal, and recovery states do not replace the application shell.

The page coordinator is `frontend/src/pages/assistant.tsx`. Query and turn ownership live in `frontend/src/hooks/use-assistant.ts`. Visible contracts live in `frontend/src/components/assistant/chat-thread.tsx`, `chat-composer.tsx`, `assistant-sidebar.tsx`, and the block components.

## Conversation selection

The route may select a durable conversation or the entry screen. The current conversation is the history query result's canonical ID when available, otherwise the selected route ID. Placeholder-to-durable alias adoption repairs navigation without treating that repair as a new user selection.

Selecting a sidebar row changes the active conversation and requests composer focus. Selecting New chat navigates to the empty chat screen and requests focus. New chat issues no create request. A first send allocates the local workflow placeholder and begins the Studio create turn lazily.

A stale or missing conversation returns to a usable new-chat state. It does not retain a composer bound to a nonexistent durable ID.

## Thread states

The thread distinguishes execution state from transcript content. An active turn can have no printable output yet, printable streaming output, or structured activity without text.

### Idle empty

When no turn has run and there are no messages, the thread shows:

```text
Start a new conversation
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

`[data-assistant-halo]` is the stable test marker. The halo itself is decorative; the containing status announces the state. Its fade-out remains in the DOM briefly with `aria-hidden` so exit animation does not create repeated announcements.

### Streaming before content

When the assistant message exists but its text block is still empty, the answer slot displays three streaming dots. In this placement the dots own the live-region semantics:

```html
<span
  data-streaming-dots
  role="status"
  aria-label="Assistant is answering"
>...</span>
```

`[data-streaming-dots]` is the stable test marker. Once printable content arrives, the dots leave layout immediately and fade from their last measured position, preventing the first text from shifting after render. Leaving dots are `aria-hidden`.

### Streaming content

After a text delta or another printable assistant block appears, the thread renders the actual block. Text updates in place with its streaming treatment. The thread follows the tail while the reader remains within 48 pixels of it. A deliberate upward scroll suspends automatic following; streaming must not pull the reader back down.

A newly sent user message always restores tail following so the optimistic echo is visible.

### Settled

Completed, blocked, failed, and cancelled turns stop thinking and streaming treatment. Open text and run activity are finalized according to the terminal. A blocked turn keeps its recovery card visible. A cancelled turn reflects the user's Stop action and does not show the empty-turn error solely because no assistant text arrived.

### Empty terminal

If a noncancelled turn closes without printable content, and no transcript projection is still in flight, the UI waits 700 milliseconds before rendering:

```html
<p role="alert" data-empty-turn-error>
  Sorry, there seems to be an error with the request for now.
</p>
```

The grace period is longer than the 500-millisecond thinking exit and protects against status and transcript events arriving in opposite orders. A fresh episode resets the timer in render state, so an old settled flag cannot leak into a new turn.

`turnPrinted` from the current stream episode is authoritative when available. After reload, the visible assistant tail is the fallback evidence. Earlier content in an approval continuation cannot be mistaken for content printed by the new episode.

### Transcript settling

An active transcript fetch or projection deadline suppresses the empty-turn error. A terminal can arrive before its history/query projection; the UI keeps the thread coherent until the pump reports whether the episode printed and projection either lands or completes.

## Stable semantic markers

The browser suite intentionally asserts three markers rather than CSS implementation classes:

| Marker | Meaning |
| --- | --- |
| `[data-assistant-halo]` | active thinking identity treatment, decorative inside a named status |
| `[data-streaming-dots]` | answer pending at the future content position |
| `[data-empty-turn-error]` | closed noncancelled episode with no printable content |

These attributes are part of the testing contract. They can move with equivalent markup but must not be removed without updating both component tests and Playwright helpers.

## Message grouping and blocks

Consecutive messages with the same role render as one visual group. Aevatar can split one assistant turn into text and activity messages; grouping keeps them under one assistant identity without changing their underlying message and block IDs.

Supported blocks are:

- `text`, rendered as formatted assistant text;
- `run`, rendered as tool/workflow activity;
- `approval_card`, rendered as a human decision card;
- `connect_card`, rendered as credential recovery;
- `action_card`, rendered as a v4 browser action;
- `artifact`, rendered as media or downloadable output.

An empty leading text block is not rendered and does not count as printable content. An unknown block or message schema renders a bounded `Unsupported assistant content` shell rather than throwing the thread or silently disappearing.

Structured activity is attached to one synthetic assistant activity message per turn. Card updates patch the existing block. They do not append duplicate cards for state changes.

## Optimistic user messages

The composer clears before `onSend` resolves. The page therefore carries a pending send echo from the start of the mutation until the transport projection contains the corresponding new user message.

The echo identity is count-aware. An older identical user message does not suppress the pending one; the number of matching messages must advance beyond the send-time snapshot. This keeps repeated prompts visible without duplicating the current prompt after projection lands.

The optimistic echo covers both:

- first send, while the local conversation and query identity are being created; and
- existing-conversation send, during the shorter gap before the transport emits the user message.

On send failure, the composer restores the trimmed submitted message and schedules it as the draft. The transcript does not retain a false optimistic user turn after the mutation fails.

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

Stop requests cancellation through `useCancelTurn`. Typed runs use their server turn identity when available. Workflow cancellation aborts the client stream and uses create recovery when a first-turn durable identity may have been created upstream.

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

The transport tombstones the durable identity during deletion, blocks sends and continuations, and prevents late stream or create-recovery adoption. The UI navigates to a safe remaining or empty state when the active conversation is deleted.

Implementation: `frontend/src/components/assistant/assistant-sidebar.tsx`, `frontend/src/pages/assistant.tsx:handleDelete`, and `frontend/src/lib/assistant/aevatar-transport.ts:deleteConversation`.

## Loading and fetch failures

When a selected conversation is loading and no messages are available, the chat surface shows:

```text
Loading conversation...
```

It does not briefly show the new-conversation empty state.

A transcript read failure appears as a status notice above the still-usable thread and composer. The notice explains that new messages remain available. It does not replace the entire chat surface because actor/workflow streaming and history materialization are different upstream resources.

Conversation-list transport failures use the shared assistant transport error reporting. Query keys remain scoped to the current authenticated user and transport instance so stale data does not cross auth changes or mock/real transport switches.

## Accessibility contract

The surface exposes these stable roles and names:

| Element/state | Role or accessible name |
| --- | --- |
| thinking row | `role=status`, `Assistant is thinking` |
| pre-content answer dots | `role=status`, `Assistant is answering` |
| empty terminal notice | `role=alert` |
| transcript fetch failure | `role=status` |
| composer | textbox |
| send button | `Send message` |
| stop button | `Stop assistant turn` |
| chats list | `navigation`/`nav` |
| conversation row | button named by conversation title |
| row menu trigger | `Options for {title}` |
| delete confirmation | dialog with title `Delete chat?` |
| active run ledger | `role=status`, `Tool activity` |
| artifact download | `Download {artifact name}` |

Decorative icons, halo sprites, and dot children are hidden from assistive technology. Live roles are removed or hidden during exit transitions. Action and approval components expose their own controls through native buttons and dialog semantics.

## Rendering continuity

The page must not render incoherent overlaps or empty gaps during state handoff. In particular:

- the optimistic user echo covers conversation allocation;
- thinking begins before a transcript query identity exists;
- dots occupy the future answer slot;
- leaving loaders exit out of layout;
- empty-turn error waits for projection and exit animation;
- loading detail does not show the empty-chat prompt;
- active streams preserve local transcripts while list metadata refreshes;
- scroll following respects deliberate reader position; and
- fixed message, card, toolbar, and composer dimensions prevent dynamic content from shifting controls.

The Playwright continuity probe described in [Testing and gaps](07-testing-and-gaps.md) observes DOM mutations to catch transient states that ordinary final assertions would miss.

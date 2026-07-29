# Assistant Chat Frontend UI Specification

Status: implementation contract, 2026-07-29

This document is the source of truth for the NyxID assistant chat frontend and
its in-thread action cards. It specifies what the browser renders, how the user
interacts with it, and the frontend lifecycle around those interactions.

The live Aevatar wire contract remains authoritative for transport details. If
this document and the verified contract disagree, follow the live contract and
update this document. See `chat-action-cards-brief.md` for the contract evidence
and `ASSISTANT_STREAM_ALIGNMENT.md` for deployment and streaming diagnostics.

## 1. Purpose and scope

The assistant is a working product surface, not a marketing page. It must let a
user:

- create, select, and delete conversations;
- send text and watch the assistant stream a response;
- inspect tool progress, artifacts, connection blockers, and approvals inline;
- complete browser-owned actions without leaving the conversation; and
- understand whether an action is pending, running, completed, declined, or
  failed without reading transport events.

This specification covers `/assistant`, the shared shell, sidebar, thread,
composer, content-block rendering, approval cards, and browser action cards. It
does not redefine backend authorization, the Aevatar event taxonomy, or the
dashboard's Add Service journey.

## 2. Design direction

The chat uses the main NyxID design system in `DESIGN.md`.

- Surfaces are neutral: `bg-background`, `bg-card`, `bg-muted`, and semantic
  border tokens provide structure.
- Color is earned. Purple marks NyxID identity and a pending user interaction;
  green means success or approval, red means denial or failure, and amber is
  reserved for genuine warnings or urgent timing.
- Pending cards must not look like warnings. Do not use a yellow wash or
  warning-tinted card body for a normal approval or connection request.
- The selected pending accent is warm NyxID purple. Do not use blue for the
  approval-card header or add a blue top rail.
- Cards are sectioned, compact, and operational. They use a neutral body,
  explicit dividers, small type, and stable icon/button dimensions.
- Rich cards are not nested inside decorative parent cards. They render as
  direct content blocks in the assistant answer column.
- Both light and dark themes must preserve text, badge, icon, border, and action
  contrast. Use theme-aware component variants rather than raw one-theme text
  colors for badges.

## 3. Surface anatomy

### 3.1 Shell

`AssistantShell` owns a full-height `h-dvh` layout with safe-area insets and no
document-level scrolling.

- Header: 52 px high, bottom border, NyxID logo on desktop, breadcrumb-like
  current conversation title, theme toggle, and user menu.
- Desktop sidebar: fixed 200 px width from the `md` breakpoint upward.
- Main region: fills remaining width and clips overflow so the thread controls
  its own scroll position.
- Mobile sidebar: opened by the menu icon as an 88vw drawer capped at 320 px.
  It has a dismissing scrim, close button, Escape handling, and closes after a
  navigation or button action.
- Account actions: Settings and Log out remain available through the header
  menu. The sidebar footer links back to Studio.

### 3.2 Sidebar

The sidebar is a dense navigation surface with three regions:

- `New chat`: the dominant purple action.
- Workspace: Plugins and Approvals are real routes; unimplemented destinations
  remain visibly disabled and expose a `Coming soon` tooltip.
- Chats: scrollable conversation list with one active row. The active icon is
  purple while inactive and hover surfaces remain neutral.

Conversation deletion requires a confirmation popover. A failed delete keeps
the action available and surfaces an error; a successful delete removes the row
and clears a stale `?c=` selection when necessary.

### 3.3 Thread and composer

The thread is the only vertically scrolling chat region. Its content column is
capped at 680 px and centered with responsive side padding. New messages,
thinking state changes, and streamed deltas scroll to the latest content.

The composer is a separate 728 px centered region below the thread. It provides:

- a textarea with a 32,768 character limit;
- Enter to send and Shift+Enter for a new line;
- a purple icon-only Send control;
- an icon-only Stop control while a turn is active;
- disabled editing during an active turn; and
- restoration of the submitted text when sending fails.

The first send from an empty state creates a conversation before streaming and
navigates to it. Transport failures are surfaced as toasts, not swallowed by
the composer.

## 4. Message presentation

Consecutive messages with the same role form one visual group. This prevents
Aevatar's text, tool activity, and follow-up fragments from repeating identity
chrome inside one turn.

- User groups are right aligned in a subtle neutral bubble with no avatar.
- Assistant groups have no outer bubble. They render beside a compact NyxID
  identity tile so rich blocks can use the full content width.
- Assistant identity placement uses a container query, not a viewport query.
  At 680 px or wider it sits in the left gutter; in a narrower chat region it
  stacks above the answer.
- Timestamps use `HH:MM` monospace text and reveal on hover or keyboard focus.
- Before the first assistant block arrives, three animated dots communicate
  thinking. Streaming text ends with a purple caret.
- A thin, pointer-transparent fade at the bottom visually joins the thread to
  the composer without blocking the final card's controls.
- Messages with an unsupported schema version and unknown content blocks render
  a neutral `Unsupported assistant content` fallback.

## 5. Content-block model

`ChatThread` renders the `ContentBlock` discriminated union from
`frontend/src/types/assistant.ts`.

| Block type | Component | Responsibility |
|---|---|---|
| `text` | `TextBlock` | Render assistant markdown and streaming caret |
| `run` | `RunCard` | Show ordered tool/run progress and terminal state |
| `artifact` | `ArtifactBlock` | Show file metadata, preview, and download action |
| `connect_card` | `ConnectCard` | Represent `nyxid.authorization.required` blockers |
| `approval_card` | `ApprovalCard` | Collect an approve or deny decision for a tool gate |
| `action_card` | `ActionCard` | Complete or decline a browser-owned requested action |

Block components receive callbacks; they do not call the Aevatar transport
directly. `AssistantPage`, `use-assistant.ts`, and the selected transport own
mutation, turn, and cache state.

## 6. Shared rich-card language

Interactive pending cards use this anatomy:

1. A neutral `bg-card` outer surface with `border-border`, `rounded-xl`, clipped
   overflow, and a restrained shadow.
2. A compact header containing a fixed icon tile, 13 px title, short supporting
   copy, and a status badge.
3. One or more neutral sections separated by `border-border` dividers.
4. A `bg-muted` footer containing the available actions.
5. Terminal cards collapse to a smaller receipt instead of preserving inactive
   pending controls.

Do not encode behavior using color alone. State is always named in text and
paired with an icon where useful. Long wire-supplied identifiers and labels must
wrap, truncate, or be clamped inside the card; they must never widen the thread.

## 7. Approval card

### 7.1 Pending state

A pending approval card is neutral with a purple interaction accent:

- no top rail;
- no blue highlight;
- no yellow card fade;
- purple `ShieldAlert` icon tile using a low-opacity purple surface and border;
- `Badge variant="accent"` with the visible label `Pending`;
- heading `Approval required` and NyxID-owned review guidance;
- expiry countdown aligned with the badge; and
- neutral body and footer sections.

The purple treatment applies equally to `per_request` and `grant`. Approval mode
changes the scope copy, not the visual severity.

The scope row names the service, the redacted agent-key prefix, and either
`per-request approval` or the grant duration. A grant also explains that repeat
writes are allowed for the displayed interval. The full key, token, or secret
must never appear.

`Approve and send` uses a success treatment. `Deny` uses a destructive
treatment. Both disable while either decision is in flight, and clicks are
throttled to one decision per 750 ms.

The countdown refreshes every 30 seconds. Less than five minutes remaining is
urgent and uses destructive text. An invalid server timestamp omits the
countdown rather than displaying a fabricated value.

### 7.2 Terminal states

- `approved`: green receipt, `Approved and sent`.
- `denied`: red receipt, `Request denied`, plus `Nothing was sent.`
- `expired`: neutral receipt, `Request expired`, plus a disabled/future
  `Request again` affordance whose tooltip describes the missing wiring.
- `cancelled`: neutral receipt, `Request cancelled`, plus `Nothing was sent.`

When supplied, the decision channel renders as a neutral `via web`,
`via telegram`, or `via mobile` badge.

## 8. Browser action card

### 8.1 Accepted request

The live transport may receive an AG-UI `CUSTOM` frame named
`nyxid.action.request`. The frontend structurally validates the payload and
supports schema version 4. The v1 action registry supports only
`service.connect` with exactly one of:

- `catalogService`: service slug, requested scopes, optional node id, and
  optional target organization id; or
- `customService`: name, endpoint URL, authentication method, authentication
  key name, optional node id, and optional target organization id.

The request becomes one `action_card`, deduplicated by `actionRequestId` within
the origin turn. Re-emitting the same request updates the existing block rather
than adding another card.

Unsupported verbs, unsupported schema versions, or ambiguous/missing parameter
variants render an `Unsupported action request` card. It has no completion CTA
but retains Decline so the user can unblock the model safely.

### 8.2 Pending and in-progress presentation

Supported pending cards use:

- **no top accent rail** — no colored bar or gradient strip on any card edge, in any
  state (design decision 2026-07-29: edge accents read as generic AI output). The
  pending affordance is the purple icon tile plus the accent badge, matching the
  approval card;
- neutral card sections and footer;
- a service icon or custom endpoint icon;
- an `Action required` purple accent badge;
- NyxID-owned consent copy from the local action registry;
- visible service, scope, route, and organization summaries; and
- a purple primary CTA plus a neutral Decline action.

The model controls identifiers and parameters, not consent prose. Any
interpolated service label is stripped of control characters, collapsed to one
line, and clamped to 32 characters. A custom endpoint may display only a safely
parsed hostname. URLs with embedded credentials are rejected from the prefill
path and must never be rendered.

Opening the journey immediately marks the card `in_progress`, replaces the
status with `In progress`, and disables both card actions. Closing the dialog
before completion returns the card to `pending`; it does not report a decline.

### 8.3 Add Service journey

The CTA opens the existing `AddKeyDialog` rather than implementing a second
credential form.

- Catalog request: pass `prefillSlug`.
- Custom request: pass name, endpoint URL, auth method, and auth key name through
  `prefillCustom`.
- Both variants: pass the optional node and target organization defaults.
- Catalog `requestedScopes` are displayed in the action card but are not passed
  into the dialog. `AddKeyDialog` has no requested-scope prefill contract; the
  existing service/OAuth flow remains authoritative for selectable and granted
  scopes. The UI must not imply the displayed request grants scopes by itself.

`AddKeyDialog.onSuccess` fires only after the user reaches the post-connect
verification step and chooses Done. The dialog returns
`{userServiceId: createdKey.id}`. The action report includes that id as
`resource.userService.userServiceId`. If a future completion path cannot return
a valid user-service id, completion may omit `resource`; it must never substitute
a key, credential, URL, or guessed identifier.

### 8.4 Resolution and receipts

Decline reports `declined` immediately without opening the dialog. Completion
reports `completed` after the dialog's success callback. The component guards
against duplicate resolution calls.

- `completed`: compact green `Service connected` receipt.
- `declined`: compact neutral `Action declined` receipt.
- `failed`: compact red `Connection failed` receipt.

Receipts show only a safe outcome note and clamped service label. Secrets and
full credential-bearing URLs are forbidden in cards, reports, logs, and tests.

## 9. Transport and lifecycle boundaries

### 9.1 Text turn

The conversation stream endpoint receives an exact allowlisted body:

```json
{
  "type": "text",
  "prompt": "<user text>",
  "clientRequestId": "<stable id>"
}
```

Do not add unknown JSON members. A retry reuses the same `clientRequestId`.

### 9.2 Action continuation

Resolved actions post to the same conversation stream endpoint with an exact
allowlisted body:

```json
{
  "type": "action.continue",
  "clientRequestId": "<stable retry id>",
  "originTurnId": "turn-...",
  "actions": [
    {
      "actionRequestId": "act-...",
      "originTurnId": "turn-...",
      "disposition": "completed",
      "resource": {
        "userService": {
          "userServiceId": "<created service id>"
        }
      }
    }
  ]
}
```

The body must not include `prompt`, `inputParts`, or any UI-only fields. Reports
are grouped by `originTurnId`; a batch is non-empty, contains no duplicate action
request ids, and never mixes origins.

If a local turn is active, resolved reports stay queued. The continuation starts
only after the conversation becomes idle and streams through the same SSE,
cursor, retry, and watchdog path as a text turn.

A batch is settled only when the continuation proves it ran: `RUN_FINISHED`,
`RUN_STOPPED`, or a reached approval gate. Any error terminal, cancellation,
network failure, or watchdog stall requeues the batch with the same
`clientRequestId` for the next idle opportunity.

Do not look for continuation admission reason codes on the continuation stream.
Aevatar publishes `nyxid.continuation.changed` on the origin-turn session, not
the new continuation session. Unknown custom frames remain ignored.

### 9.3 Page lifecycle

The active turn is conversation-scoped. The composer and Stop action reflect
`running` and `waiting` states. Approval continuations and action continuations
update the same thread reducer and preserve cursor monotonicity.

Action cards currently live in the in-memory conversation projection. Aevatar
history is text-only, so action cards are not rehydrated after reload. The next
text turn allows the server to re-emit a still-pending action idempotently.

## 10. Mock and demo contract

`?mock` must demonstrate the complete UI without Aevatar:

- streamed assistant text and tool/run blocks;
- one pending approval card in the Stripe conversation;
- an action request for a catalog service such as GitHub;
- Add Service success changing the card to a completed receipt;
- Decline changing the card to a declined receipt;
- a continuation response appended to the same thread; and
- workspace counts that include only genuinely pending approvals.

Mock behavior must preserve the same content-block states and callback ordering
as the live transport. It must not become a permanent design-variant gallery.

## 11. Accessibility and responsive requirements

- Icon-only buttons have accessible names and unfamiliar controls have
  tooltips.
- Semantic buttons are used for every command. Disabled state is exposed while
  a request is in flight.
- Dialogs retain their existing focus trapping, labelled title/description,
  Escape behavior, and return focus.
- Text and status labels carry meaning independently of color.
- Keyboard focus can reveal timestamps and operate every card action.
- IDs, hostnames, scopes, service names, and footer text must not overflow the
  680 px thread or a 320 px mobile viewport.
- Card actions may wrap on mobile without resizing icons or losing their click
  targets.
- The shell, thread, pending cards, terminal receipts, and Add Service handoff
  must be checked in both light and dark themes.

## 12. Acceptance criteria

The frontend is acceptable when all of the following hold:

1. A user can create a conversation, send text, stop a live turn, switch chats,
   and delete a chat with confirmation.
2. Consecutive same-role messages group correctly and streaming feedback remains
   visible before and during the first text block.
3. Every known content block renders through its dedicated component and unknown
   content degrades safely.
4. Pending approval cards are neutral and sectioned with a purple icon/badge,
   no top rail, no blue header highlight, and no yellow body fade.
5. Per-request and grant approval cards share the same purple treatment while
   preserving their distinct scope copy.
6. Supported action cards open the correctly prefilled `AddKeyDialog`; closing
   the dialog restores pending, completion reports once, and decline never opens
   the dialog.
7. Unsupported action requests remain declineable and never expose a fake CTA.
8. Text and continuation request bodies contain exactly the verified wire keys.
9. Reports queue behind active turns, deduplicate, batch only by origin, and
   retain their request id after any unproven continuation outcome.
10. Desktop and mobile layouts have no horizontal overflow in light or dark
    theme.
11. `npm run build`, `npm run test`, and `npm run lint` pass from `frontend/`.

## 13. Explicit non-goals

- A backend action registry or new NyxID backend endpoint.
- New action verbs beyond `service.connect`.
- Action-card persistence in Aevatar chat history.
- Standing grants, remember-me behavior, or scope-widening UI.
- Forwarding model-supplied consent copy.
- Adding scope-prefill behavior to `AddKeyDialog` without a separate product and
  contract decision.
- Using action cards as general alerts or warning banners.

## 14. File map

| Area | Primary files |
|---|---|
| Route composition | `frontend/src/pages/assistant.tsx` |
| Shell and navigation | `frontend/src/components/assistant/assistant-shell.tsx`, `assistant-sidebar.tsx` |
| Thread and composer | `frontend/src/components/assistant/chat-thread.tsx`, `chat-composer.tsx` |
| Rich cards | `frontend/src/components/assistant/blocks/action-card.tsx`, `approval-card.tsx`, `connect-card.tsx`, `run-card.tsx`, `artifact-block.tsx` |
| Content and turn types | `frontend/src/types/assistant.ts` |
| Action validation | `frontend/src/schemas/assistant-actions.ts`, `frontend/src/lib/assistant/action-registry.ts` |
| Live transport | `frontend/src/lib/assistant/aevatar-transport.ts`, `stream.ts`, `sse.ts` |
| Mock transport | `frontend/src/lib/assistant/transport.ts`, `mock-data.ts` |
| UI state hooks | `frontend/src/hooks/use-assistant.ts` |
| Existing connect journey | `frontend/src/components/dashboard/add-key-dialog.tsx` |

---

# Part II — Action-card catalogue and showcase (target designs)

Author: Fable (PM), 2026-07-29. Part I is the implementation contract for what is shipped.
Part II specifies the **target design for every verb in the initial "NyxID ↔ Aevatar —
Action Contract" (Schema v3, 2026-07-24)** — the cards the chat *will* show as the verb
surface grows — and the static showcase page that renders all of them for design review.
Nothing in Part II overrides Part I's non-goals: only `service.connect` is implemented
today; every other row below is a design target, and its risk tier is a PM-proposed
default until the backend action registry exists and becomes authoritative.

## 15. Risk decides the card form (the "why")

| Risk | Interaction | Card form | Rationale |
|---|---|---|---|
| `low` | Execute immediately, show a receipt. No prompt. | **Receipt card**: outcome title + safe note + reference chips + microtag "Executed immediately — no confirmation" | Config-level changes; prompting here trains reflexive clicking and spends the attention needed for prompts that matter |
| `grant` | One confirmation — the CTA and its journey. | **Action card** (Part I §8 anatomy): neutral surface, purple icon tile + accent badge. No top accent rail (no card has one, any state). | Changes what someone or something can reach, or collects/reveals a credential |
| `destructive` | Confirmation every time, never remembered. | **Action card**: neutral surface, red icon tile + destructive badge, warning line "This cannot be undone. You will be asked every time." No rail. | Irreversible; the repeat confirmation is the point |

Footer microcopy by class: secret-bearing grant → "Nothing is shared until you finish.";
other grant → "One confirmation."; destructive → "Asked every time. Never remembered."

Remember-me is a server-side approval grant, never a client checkbox. `key.extend_scope`
and `key.bind_credential` are **never** remember-eligible (self-escalating — exactly what
a prompt-injected agent would want pre-approved), plus everything destructive.

## 16. Data presentation rules (the "data")

Extends Part I §6/§8.2; these rules bind every card in §17:

- Chip rows: 10px uppercase label + badges. Slugs, scopes, ids, hosts, user codes →
  `secondary` badges; ids/scopes/codes mono; node refs → `info` badge with server glyph;
  every chip `max-w-full truncate`.
- Scopes render one chip per scope, mono.
- Custom endpoints render **host only**; invalid or credential-embedding URLs render
  nothing.
- Patches (`*.update` verbs) render changed-field chips: field name always; the new value
  only when it is a non-credential scalar (name, rate limit, mode); otherwise the field
  name alone.
- `device.approve` renders the user code grouped mono (`XXXX-XXXX-XXXX`) — safe because
  the user already holds it; the journey re-verifies through the device preview flow.
- One-time reveals (new keys, registration tokens, SA secrets, client secrets) happen
  **inside the journey modal only**. The chat transcript is long-lived and re-readable;
  no reveal, secret, password, WiFi credential, full redirect URI, or raw patch ever
  renders in a card or receipt.

## 17. Card inventory — every verb in the initial contract

Columns: **Risk** (→ card form per §15) · **Journey** (what the CTA opens; `—` = low-risk
receipt, no CTA) · **Card shows** (data per §16) · **CTA**. Example values in the showcase
are the contract doc's own (`api-github`, `repo`, `k_1a2b`, `n_77`, `sa_9`, `cl_5`,
`us_44`, `XXXX-XXXX-XXXX`).

### 17.1 V1 — secret-bearing browser flows (all `grant`; the journey owns the secret)

| Verb | Journey | Card shows | CTA |
|---|---|---|---|
| `service.connect` (catalog) — **shipped** | AddKeyDialog multistep | Service (icon + label from slug), scopes, node ref?, org ref? | Connect {Service} |
| `service.connect` (custom) — **shipped** | AddKeyDialog custom path | Name (clamped), endpoint host, auth method + key name, node?, org? | Connect {Name} |
| `service.reauthorize` | AddKeyDialog reconnect | Key ref, requested scopes | Re-authorize |
| `provider.set_app_credentials` | Credentials form modal (client_id + secret entered in-modal) | Provider slug | Set app credentials |
| `key.create` | Key-create modal; key revealed once in-modal | Name, platform, allowed services | Create key |
| `key.rotate` | Rotate + one-time reveal modal | Key ref | Rotate key |
| `node.register_token` | Token mint + one-time reveal + setup instructions | Node name (`[a-z0-9-]{1,64}`) | Create registration token |
| `node.rotate_token` | Rotate + one-time reveal | Node ref | Rotate token |
| `node.inject_credential` | Credential push wizard (value typed in-modal) | Node ref, service slug | Inject credential |
| `service_account.create` | SA create; secret shown once in-modal | Name, allowed scopes | Create service account |
| `service_account.rotate_secret` | Rotate + one-time reveal | SA ref | Rotate secret |
| `developer_app.create` | App create; client_secret shown once in-modal | App name, redirect hosts | Create app |
| `developer_app.rotate_secret` | Rotate + one-time reveal | Client ref | Rotate secret |
| `account.mfa_setup` | MFA multistep (QR → confirm code) | — (account-level; body copy says so) | Set up MFA |
| `device.approve` | Device preview → approve (two explicit clicks, anti-phishing block per Critical Rule 12) | User code, grouped mono | Review device |
| `device.onboard` | Onboarding wizard (WiFi + QR in-modal, never echoed) | Device label | Onboard device |

### 17.2 V2 — services

| Verb | Risk | Journey | Card shows | CTA |
|---|---|---|---|---|
| `service.update` | low | — | Service ref + changed fields | — |
| `service.delete` | destructive | Confirm dialog | Service ref + name | Delete service |
| `service.route` | low | — | Service ref, node ref | — |
| `service.add_ssh` | grant ⚠︎body-unconfirmed | SSH add wizard | Host, principal, auth mode | Add SSH service |
| `service.convert_ssh` | grant | Confirm dialog | Service ref | Convert to SSH |
| `service.rotate_credential` | grant | Rotate journey | Service ref | Rotate credential |

### 17.3 V2 — keys and credentials

| Verb | Risk | Journey | Card shows | CTA |
|---|---|---|---|---|
| `key.update` | low | — | Key ref + changed fields | — |
| `key.delete` | destructive | Confirm dialog | Key ref + name | Delete key |
| `key.extend_scope` | grant 🔒never-remember | Confirm dialog | Key ref, **added** services | Extend scope |
| `key.bind_credential` | grant 🔒never-remember | Confirm dialog | Key ref, service, credential **label** | Bind credential |
| `external_key.rotate` | grant | Rotate journey | Key ref | Rotate |
| `external_key.delete` | destructive | Confirm dialog | Key ref | Delete |
| `external_key.add_gcp_service_account` | grant | SA-JSON paste journey (in-modal) | Provider slug | Add GCP service account |
| `connection.revoke` | grant | Confirm dialog | Service ref | Revoke connection |
| `provider.disconnect` | grant | Confirm dialog | Provider slug | Disconnect |

### 17.4 V2 — approvals

| Verb | Risk | Journey | Card shows | CTA |
|---|---|---|---|---|
| `approval.decide` | grant | Opens the approval decision UI | Request ref (details from the approvals read surface) | Review request |
| `approval.configure` | low | — | Service ref, mode | — |
| `approval.enable` | low | — | — | — |
| `approval.disable` | grant | Confirm dialog (widens agent autonomy) | — | Disable approvals |
| `approval.revoke_grant` | grant | Confirm dialog | Grant ref | Revoke grant |

### 17.5 V2 — nodes

| Verb | Risk | Journey | Card shows | CTA |
|---|---|---|---|---|
| `node.delete` | destructive | Confirm dialog | Node ref + name | Delete node |
| `node.transfer` | destructive | Confirm dialog | Node ref, target owner ref | Transfer node |
| `pending_credential.push` | grant | Push wizard | Node ref | Push credential |
| `pending_credential.inject` | grant ⚠︎body-unconfirmed | Inject wizard | Node ref | Inject |
| `pending_credential.cancel` | low ⚠︎body-unconfirmed | — | Node ref, pending ref | — |

### 17.6 V2 — organisations

| Verb | Risk | Journey | Card shows | CTA |
|---|---|---|---|---|
| `org.create` | low | — | Org name | — |
| `org.update` | low | — | Org ref + changed fields | — |
| `org.delete` | destructive | Confirm dialog | Org ref + name | Delete organisation |
| `org.join` | grant | Confirm dialog | Invite ref (mono) | Join organisation |
| `org.set_primary` | low | — | Org ref | — |
| `org.member_add` | grant | Confirm dialog | Org ref, user ref, role | Add member |
| `org.member_update` | grant | Confirm dialog | Org ref, member ref, role | Change role |
| `org.member_remove` | grant | Confirm dialog | Org ref, member ref | Remove member |
| `org.invite_create` | grant | Invite journey | Org ref, role, scope source?, TTL? | Create invite |
| `org.invite_cancel` | low | — | Org ref, invite ref | — |
| `org.role_scope_set` | grant | Confirm dialog | Org ref, role (path key), scopes | Set role scopes |
| `org.role_scope_clear` | grant | Confirm dialog | Org ref, role | Clear role scopes |

### 17.7 V2 — account

| Verb | Risk | Journey | Card shows | CTA |
|---|---|---|---|---|
| `account.profile_update` | low | — | Changed fields | — |
| `account.revoke_consent` | grant | Confirm dialog | Client ref | Revoke consent |
| `account.delete` | destructive | Typed-confirmation journey | — (account-level) | Delete account |

### 17.8 V2 — endpoints, service accounts, developer apps

| Verb | Risk | Journey | Card shows | CTA |
|---|---|---|---|---|
| `endpoint.update` | low | — | Endpoint ref + changed fields | — |
| `endpoint.delete` | destructive | Confirm dialog | Endpoint ref | Delete endpoint |
| `service_account.update` | low | — | SA ref + changed fields | — |
| `service_account.delete` | destructive | Confirm dialog | SA ref + name | Delete service account |
| `service_account.revoke_tokens` | grant | Confirm dialog | SA ref | Revoke tokens |
| `developer_app.update` | low | — | Client ref + changed fields | — |
| `developer_app.delete` | destructive | Confirm dialog | Client ref + name | Delete app |

### 17.9 V2 — notifications and integrations

| Verb | Risk | Journey | Card shows | CTA |
|---|---|---|---|---|
| `notifications.update` | low | — | Changed fields | — |
| `notifications.telegram_link` | grant | Telegram link journey | — | Link Telegram |
| `notifications.telegram_disconnect` | low | — | — | — |
| `openclaw.connect` | grant | Gateway connect journey (token in-modal) | Gateway host | Connect OpenClaw |

### 17.10 V2 — service pools (⚠︎ prerequisite: no pools dashboard page exists yet)

| Verb | Risk | Card shows | CTA |
|---|---|---|---|
| `pool.create` | low | Slug, name | — |
| `pool.delete` | destructive | Pool ref | Delete pool |
| `pool.add_member` / `pool.remove_member` | low | Pool ref, service ref | — |
| `pool.set_strategy` | low | Pool ref, strategy | — |

### 17.11 Deep link

| Verb | Risk | Card shows | Notes |
|---|---|---|---|
| `admin.open` | low | Route label (NyxID-owned allowlist), params summary | ⚠︎ the route_key allowlist does not exist; unimplementable until defined. Renders as a receipt-style card with an "Open in dashboard" link |

**Tag legend:** ⚠︎body-unconfirmed = contract §13 (HTTP body not confirmed — read the
handler before building the journey); 🔒never-remember = excluded from standing grants;
prerequisite = blocked on a missing dashboard page.

## 18. Showcase page — `docs/assistant-action-cards-showcase.html`

**What:** one self-contained static HTML file rendering *every* card in §17 plus the six
Part I §8 states, in the shipped visual language, so the whole catalogue is reviewable
side by side without running the app.

**Why:** §17 is only reviewable as words; design review needs to see all ~74 cards in
both themes before the registry and journeys are built. The page is a design artifact —
not production code, never imported by the app.

**Requirements:**

1. Single file, opens from `file://`, no build step. Only external fetch: the same Google
   Fonts stylesheet the app loads (Mona Sans + JetBrains Mono); must degrade to system
   fonts offline.
2. **Data-driven:** one JS array of card descriptors (verb, tier, risk, tags, title, body
   copy, CTA, chip rows with the §17 example values) + one render function per card form
   (action card, receipt). No hand-written per-card HTML blocks.
3. Tokens must be transcribed from `frontend/src/app.css` (the live source of truth) as
   CSS custom properties — dark values default, light values behind a working theme
   toggle. Pending accent = purple icon tile + badge on `--color-nyx-secondary-400`
   (#A672FB); primary CTA gradient `#A672FB → #5E00F5`; badge recipes per
   `components/ui/badge.tsx`. **No top accent rails on any card, any state.**
   Chat-column layout, max-width 680px, with section headers per §17 group carrying the
   group's one-line why.
4. A **States** section first: `service.connect` in all six states (pending, in_progress,
   completed, declined, failed, unsupported) exactly as shipped in `action-card.tsx`.
5. A **Flow** section immediately after States: the continuous wizard-style journey,
   rendered as a numbered storyboard so every stage is visible without interaction
   (an optional click-through walkthrough may sit on top, but the storyboard is the
   requirement). It answers "what does the CTA actually lead to, and what pings Aevatar."

   **Fidelity rule (hard):** every modal frame must be a faithful mockup of a real
   `WizardStep` state in `components/dashboard/add-key-dialog.tsx` (+
   `connect-verify-step.tsx`), using the REAL `StepHeader` title/description strings
   from the code — quoted below, verified 2026-07-29. The real dialog has **no numeric
   step indicator** ("1 of 3" does not exist — do not invent one); its chrome is the
   `Dialog` shell + `StepHeader` (service icon, title, description) + step body + Back.
   Machine facts that the storyboard must respect: on `prefillSlug` the **catalog step
   is skipped** (`handleSelectCatalog` auto-advances to `routing`); the
   managed-vs-Custom-App OAuth client choice is **inline on the routing screen**;
   `handleRoutingDirect` routes oauth2 → `oauth` (managed) / `oauth_credentials` (BYO),
   device_code providers → `device_code`, key-paste entries → `form`; "Via credential
   node" → `node_setup`; every auth completion lands on `verify`.

   **Primary path** (managed OAuth, GitHub example — `req_7`, `api-github`, scopes
   `repo`):
   1. **Card pending** — the shipped pending card. Caption: CTA opens `AddKeyDialog`
      with `prefillSlug="api-github"`; catalog step skipped.
   2. **Modal · `routing`** — StepHeader "Configure routing for GitHub" + entry
      description. Body: routing choice **Direct** (selected) vs **Via credential
      node** (lists online nodes), the inline OAuth client source choice
      (**NyxID-managed** selected vs **Custom App**), Back + Continue.
   3. **Modal · `oauth`** — StepHeader "Connect to GitHub" / "This service uses OAuth
      to authenticate. Click the button below to connect your account." Body: the
      scope picker pills (`repo` preselected from the request; platform-allowlist
      gating on the managed path), the Connect button (opens the provider popup —
      caption: the token lands NyxID-side, never in the chat), Back.
   4. **Modal · `verify`** — `ConnectVerifyStep`, the aha-moment step: StepHeader
      "GitHub connected"; body per its real sub-machine (`idle → minting →
      probe_running → probe_success | mint_failed | probe_failed`): agent-key panel,
      env snippet, live probe diagnostic (the first real 200 through the broker),
      actions **Create Agent Key** / **Maybe later** / **Done**. Caption: Done (and
      Maybe later — the UserService already exists) fires
      `onSuccess({userServiceId})`.
   5. **Card completed** — the receipt state, and directly beneath it the
      **auto-continue event row**: `→ action.continue posted · req_7 completed ·
      resource us_44`, annotated "automatic — no user action", followed by the
      assistant's follow-up text bubble showing the conversation resuming.
   6. **Branches**, each its own frame: (a) **OAuth error** — the `oauth` step's
      real inline error + retry (recoverable, stays in the modal); (b) **failed** —
      the card's terminal `failed` receipt + auto-continue row
      (`disposition: failed`); (c) **declined** — Decline on the card, no modal:
      declined receipt + auto-continue row (`disposition: declined`).

   **Other real modalities** — a compact secondary strip, one faithful frame each,
   real step headers quoted:
   - **`device_code`** ×3 sub-states (`DeviceCodeStep`): configure — "Connect to
     {Service}" / "This service uses a device code to authenticate. Click continue to
     request a code." (with scope picker where supported); requesting — spinner,
     "Requesting code from {Service}..."; success — "{Service} connected" / "Your
     {Service} account has been connected successfully."
   - **`form`** (API-key paste, e.g. `api-github-pat`) — "Configure {Service}".
   - **`node_setup`** — "{Service} — Node setup" / "Configure credentials on your
     node agent."
   - **`oauth_credentials`** (Custom App / BYO) — the client-ID + client-secret form
     reached from the routing screen's Custom App choice.
   - **`catalog`** — the grid as the entry point when there is **no** usable prefill
     (unknown slug falls back to manual pick).
6. Every card carries its verb as a mono caption, its risk as the badge, §17 tags as small
   chips, and a shipped/target marker (`service.connect` ×2 + states = shipped; all else
   target).
7. Header shows counts (total, per risk tier) **computed from the data array** — the
   completeness check, never hardcoded.
8. Icons: small inline SVG set (lucide-style, stroke currentColor). No icon fonts, no
   external images.
9. Example ids only; no realistic secrets or tokens anywhere in the file.

**Acceptance gates (verify before done):**

- [ ] The computed card count equals the §17 inventory, and every §17 row appears exactly
      once (cross-check the data array verb-by-verb against §17).
- [ ] States section shows all six states matching Part I §8.
- [ ] Flow section shows every stage of requirement 5: the primary path (pending card →
      `routing` → `oauth` → `verify` → completed card + auto-continue row + follow-up
      bubble), the three branches (OAuth in-modal error, failed, declined), and the
      six secondary-modality frames (`device_code` ×3, `form`, `node_setup`,
      `oauth_credentials`, `catalog`) — all visible without interaction.
- [ ] Every modal frame uses the real step-machine state name and the real StepHeader
      title/description strings quoted in requirement 5 — no invented steps, no
      invented "N of M" indicators, catalog step shown as skipped on prefill.
- [ ] The auto-continue rows are annotated "automatic — no user action" and show the
      disposition (`completed` / `failed` / `declined`) and, for completed, the
      `us_44` resource ref.
- [ ] **Zero top accent rails** anywhere: no card in any section or state renders a
      colored bar/strip on any edge.
- [ ] Renders at 1440px and 390px, dark and light, with no horizontal overflow.
- [ ] Opened from `file://` in a real browser with zero console errors.
- [ ] No warning/amber styling on any pending card.
- [ ] `rg -i "secret|token|password" docs/assistant-action-cards-showcase.html` yields
      only prose about secrets ("shown once", "never rendered"), never values.
- [ ] Grant cards: purple icon tile + accent badge + CTA + Decline. Destructive: red
      tile + destructive badge + "Asked every time. Never remembered." Low: receipt
      form only, no CTA.

## 19. In-app design gallery — the demoable mock page

**What:** a frontend route, `/design/action-cards`, that renders the REAL shipped
components with seeded demo data — pixel-true by construction because it *is* the app.
This is the page to demo from; the §18 static HTML is a spec illustration of the
target catalogue and stays until an explicit teardown decision.

**Why:** the static page hand-transcribes styles and drifts; a demo must show exactly
what ships. The gallery renders only what is real — no drawn approximations.

**Requirements:**

1. Route `/design/action-cards`, registered in `router.tsx`, **dev-only** (guarded by
   `import.meta.env.DEV`; unknown route in prod builds). No auth gate in mock mode —
   follow the `/assistant` `?mock` conventions. When the `?mock` param is absent the
   page says so plainly and links to `/design/action-cards?mock`.
2. **Real components only.** `ActionCard`, `ApprovalCard`, `AddKeyDialog` — imported,
   not copied. Zero new visual CSS beyond page layout (headings, grid, spacing).
   No forked card markup: if the gallery needs a state, it builds a fixture *block*
   and hands it to the real component.
3. **Sections, all fed from one seeded fixture module**
   (`lib/assistant/gallery-fixtures.ts`, values chosen to read well in a demo —
   realistic labels, no lorem):
   a. **Action card states** — the real `ActionCard` × 6: pending (GitHub,
      `repo` scope), in_progress, completed ("Service connected", `us_44`),
      declined, failed, unsupported (unknown verb). Plus the custom-endpoint
      pending variant (name "Internal API", host `api.internal.example.com`,
      bearer + `X-Api-Key`) and a via-node + org variant (`n_77`, org ref).
   b. **Approval card states** — the real `ApprovalCard`: pending (per-request),
      pending (grant with duration), approved, denied, expired, cancelled.
   c. **Live wizard** — the pending GitHub card wired with real callbacks so its CTA
      opens the REAL `AddKeyDialog` against the mock API (catalog seeded with GitHub;
      the flow walks routing → oauth → verify exactly as `/assistant?mock` does).
      Resolution updates the card state in-page (local state; no conversation
      needed). A caption links to `/assistant?mock` for the full conversational
      flow with the automatic `action.continue`.
4. Each specimen carries a small mono caption (state name / variant); the page works
   with the app's existing light/dark theme mechanism (no bespoke toggle).
5. Tests: route module renders in mock mode with all fixture states present (happy-dom,
   colocated); fixtures obey the §16 data rules (no secrets — assert the fixture module
   contains no credential-shaped strings).

**Acceptance gates:** page loads at `/design/action-cards?mock` under `npm run dev`
with zero console errors; all §19.3 specimens visible in both themes at 1440px and
390px; the CTA opens the real dialog and a completed walk flips the card to its real
completed receipt; `npm run build`, `npm run test`, `npm run lint` green; route absent
from prod build output.


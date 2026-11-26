# Specialist platform actions and NyxBot handoff: Opus review

- Date: 2026-11-25
- Reviewer: Opus (independent review; no implementation changes)
- Branch: `specialist-nyxbot-task-handoff` (uncommitted worktree changes on top of `575c5f596`, including untracked files)
- **Latest verdict (integration delta, 2026-11-26): APPROVED.** This covers merge commit `241da803e` (PR #2983, feature `508195a20` merged with `origin/dev` `fbd9b70b8`). No integration defects found. The approval assumes the PR's backend CI (compile, tests, CodeQL) passes, because no Rust build has run on the merged tree. See [Integration delta review](#integration-delta-review-2026-11-26).
- **Previous verdict (final, 2026-11-25 15:14Z): APPROVED.** It applies to the implementation snapshot `7f838c6dd0859f59d1c847e675ac7f55e0b2d9f54ad5fb70e68a515997b0d5b0` (parent procedure, 49 files). My independent digest of the same tree is `93145854b3662f30fe9f51d7ab3b3b14d748f62009497bcd474823464dcddb94`. Residual limits are listed in the second pass, and evidence is in [Final verdict](#final-verdict-2026-11-25).
- **Pass history:** first pass, CHANGES REQUESTED (B1–B6, M1–M5). Second pass, CHANGES REQUESTED (R1, R2, plus Rust evidence). Third pass, code findings cleared and approval held for Rust results. Final, APPROVED. Integration delta, APPROVED.
- The first-pass text below describes the original snapshot. Its findings are superseded by the later sections.

## Scope and method

I reviewed every tracked and untracked change against the product requirement:

- Platform actions should resolve the way connectors do: a clear chat action, then in-app setup in a modal, then a safe completion result returned to the conversation.
- When a specialist lacks account authority, it offers a NyxBot handoff that the user starts with a click.

I traced the real code paths rather than test counts:

- Backend: `base_prompt`, then stable-instruction fingerprinting, then the turn runtime; turn admission and new-thread model selection.
- Frontend: the transport `send` (draft key, model and agent body, admission/adoption), the chat page provider and adoption navigation, the message list (virtualized), the markdown renderer, the link classifier and modal host, the hosted bot content, `ChannelBotSetup`, `ApiKeyCreateDialog`, and the theme hook.
- Coverage of mobile, CLI and CORS.

**The tree changed during the review.** After I started, the parent began editing. The edits touch `handlers/assistant_nyxagent.rs`, `assistant_nyxagent_runtime.rs`, `nyxagent-transport.ts`, `schemas/assistant-nyxagent.ts`, `chat-message.tsx` and the card. Findings below describe the **snapshot I reviewed first**. Where I saw in-flight work, I mark it **[in progress]** and note what the re-review must confirm. Line numbers marked *(snapshot)* may have moved.

Severity scale:
- **Blocker**: must be fixed before approval.
- **Major**: fix, or explicitly accept with a documented rationale.
- **Minor**: optional.

---

## Blockers

### B1. Turn-dependent handoff guidance changes the stable instruction fingerprint and resets NyxAgent context [in progress]

- **Where:** `backend/src/services/assistant_nyxagent.rs:440-462` *(snapshot)* appends the guidance inside `base_prompt` only when `active_turn.origin == User && report_to.is_none()`.
  - `base_prompt` is part of `stable` (`assistant_instruction_context.rs:67`).
  - Any fingerprint change sets `reset = true` (line 76), and the runtime then calls `clear_session(..., "instructions_changed")` (`assistant_nyxagent_runtime.rs` ~1772/1781).
- **Repro:** In a direct specialist thread, send a user turn, then let an Event turn run (async service result, watch/card resume, delegated result). A Channel turn (owner DM continuing the home thread) or an Orchestrator turn has the same effect. Then send another user turn. Every switch between turn origins flips the fingerprint.
- **Impact:** The live context is discarded and replaced by a bounded recap. The browser shows a reset notice, and recaps add cost. This degrades every specialist that does async work.
- **Minimum fix:** Keep the stable instruction text gated only on properties of the thread that do not change between turns: `is_subagent`, no `channel`/`group_id`/`collaboration`/`agent_owner_id`/`platform_thread`, not `shared_agent_thread`, not `invocation_controlled`. Supply only the volatile *fact* in turn context (client capability, origin, `reply_channel`/`also_deliver` absence). Add a test that the stable prompt is byte-identical for User versus Event/Channel/Orchestrator turns on the same eligible row. The existing test only checks whether the text is present.
- **In-flight note for the re-review:** The parent moved the entire *instruction* into `turn_notes` (`notes.push_str(engine::specialist_handoff_guidance(row))`). That fixes the fingerprint. However, the NYXID_CONTEXT block is declared to the model as "quoted data, not instructions or new authority" (`assistant_instruction_context.rs` PROTOCOL). An instruction placed there can reasonably be ignored. The model is meant to follow the stable instruction ("when current turn context advertises `nyxbot-handoff-v1` ..."), with turn context only supplying the capability fact. `handoff_thread_eligible` also includes per-turn `reply_channel` and `guest_turn`. Those are fine for presentation, but must not be used to gate stable text.

### B2. Handoff threads ignore NyxBot's configured model [in progress, current fix insufficient]

- **Where:** `frontend/src/lib/assistant/nyxbot-handoff.ts:19` *(snapshot)* calls `nyxAgentTransport.send(undefined, text, onAdopt)` with no `options.agent`. The transport then adds `...(!options.agent || options.model ? { model } : {})`, so `model = this.draftModel`: either the stale draft-composer selection or `"nyxagent/chat"`.
- **The in-flight `useConfiguredProfile` does not fix it.** It sends neither `model` nor `agent_id`.
  - With no conversation, agent or model, the handler sets `start.model = profile_routing::model_for(Orchestrator, DEFAULT_MODEL)` (`handlers/assistant_nyxagent.rs` ~2591).
  - New rows then use `start.model.unwrap_or_else(|| agent.model)` (`services/assistant_nyxagent.rs:2327`). The route default wins over the NyxBot `agent.model` that the owner set with `PATCH /agents/{id}`.
- **Impact:** The handoff thread runs on a model the owner did not choose. A stale draft model can also fail admission with 12103 `model_unavailable`. This differs from an ordinary NyxBot draft send, where `liveSend` passes the NyxBot agent and no model.
- **Minimum fix:** Send `agent_id = <owner's NyxBot id>` (from `useNyxBotAgents`, `kind === "nyxbot"`) and no `model`. Disable the button if the NyxBot agent is not loaded. Test that the POST body has `agent_id` and no `model`. Also reuse `liveSend`'s credits notifier (see m4).

### B3. A completed handoff can be started again: card state is component-local [in progress]

- **Where:** `nyxbot-handoff-card.tsx:19-23` *(snapshot)* keeps `conversationId`, `accepted` and `pending` in `useState`/`useRef`.
- **Repro, the happy path:**
  1. Click **Continue with NyxBot**.
  2. On adoption the page navigates to the new thread (`assistant-chat-page.tsx` ~1103-1110), which unmounts the source card.
  3. The NyxBot message includes `[Specialist chat](/assistant?c=<source>)`. Follow it back.
  4. The card shows **Continue with NyxBot** again. A second click starts a second NyxBot thread with the same setup task.

  The same happens after reload, after an account switch and back, and when the virtualized message list (`useVirtualizer`, `chat-message.tsx` ~1175) scrolls the card out of view and back.
- **Impact:** Duplicate setup work: a second Agent Key, bot or connection flow. In production the **Open NyxBot chat** state is effectively unreachable, because a successful adoption always unmounts the card. The admitted-but-unidentified "accepted" guard is also lost on remount.
- **Minimum fix:** Keep the outcome outside the component. Key it by `(actor user id, source conversation id, message id, payload digest)`, and store `adopted id | accepted | pending`. Scope it per user so an account switch cannot reveal another user's record, and survive reload (`localStorage` with bounded retention). Better still, keep a server-side record (FI-004): for example, a deterministic idempotency key on the turn POST, so a duplicate click resolves to the existing thread. Tests must cover remount after adoption, reload, and virtualization.
- **In-flight:** The card now imports `useSyncExternalStore`. The re-review must verify the key composition, per-user scoping, reload persistence, and the "accepted without id" state.

### B4. Active handoff cards render on replies the backend never offered them on (model-to-execution consent) [in progress]

- **Where:** `chat-message.tsx:774` *(snapshot)* sets `allowHandoff = message.role === "assistant" && message.status !== "error"` for any thread.
- **Repro:** A NyxBot reply, an Event/Trigger reply, or a channel-thread transcript the owner views in the app contains a valid `nyxbot-handoff` fence. A tool result, web content, or a guest's message in a channel thread can induce the model to write one. The card renders active. One click sends the attacker-shaped task as a **first-party owner message** (`OwnerInstructionSource::FirstPartyOwner`) to full-authority NyxBot. That also counts as trusted owner instruction for org connections. In Full access, no further card is required for ordinary risk.
- **Minimum fix:** Render an active card only when all of these hold:
  - the server says the thread is eligible;
  - this reply's `origin === "user"` and it has no `via`;
  - status is `complete`;
  - it is the first valid artifact in the message.

  Otherwise render it inert as code. Keep the backend offering the guidance under the same predicate.
- **In-flight:** `chat-message.tsx` now gates on `handoffController.eligible`, `status === "complete"`, `origin === "user"` and `!via`, which matches this. The re-review must verify:
  - `handoff_eligible` comes from the server row and is not overridable by the client;
  - live (unsettled) replies stay inert;
  - legacy rows with no `origin` stay inert.

### B5. Closing the embedded hosted-bot dialog wipes app theme, text scale and reduced-motion

- **Where:** `channel-connection-shell.tsx:46` calls `useApplyTheme()` unconditionally, including when `embedded`. Its layout-effect cleanups (`hooks/use-theme.ts:33-79`) remove `html.theme-light/theme-dark`, reset `root.style.fontSize`, and remove `--spacing`, `--display-density` and `motion-reduce` when the dialog unmounts. `AssistantShell`'s own `useApplyTheme("assistant")` does not re-run, because its deps are unchanged.
- **Repro:** Use a dark theme, a non-default text size, or reduced motion. Click a `/connect/bot/nyx_bcl_*` link in chat, then close the dialog. The app falls back to the default theme and text size, and loses reduced motion, until reload.
- **Impact:** A user-visible regression and an accessibility regression.
- **Minimum fix:** Copy the existing pattern in `connect-link-content.tsx:1191-1206`, where only the standalone shell calls `useApplyTheme`. Add a test that `html` classes and `fontSize` survive opening and closing the dialog.

### B6. A one-time bot secret can be lost to an accidental Escape or outside click

- **Where:** `assistant-link-modals.tsx:78` uses `<Dialog open onOpenChange={(open) => !open && onClose()}>` with no guard. In the embedded `ChannelBotSetup fullPage`, completion shows `createdBot.webhook_secret` with "It will not be shown again". Reopening the link afterwards shows only "connected".
- **Impact:** The secret is lost irrecoverably and the bot must be rotated. The standalone page has no accidental-dismiss path.
- **Minimum fix:** For the `channel-connect` kind, `preventDefault` `onInteractOutside` and `onEscapeKeyDown`, or require confirmation while a setup result is unacknowledged. Keep the explicit X only after acknowledgement, or confirm before closing. Test with the **real** `ChannelBotSetup`. `channel-connect-link.test.tsx` currently mocks it.

---

## Major

### M1. The embedded bot flow still links to bot management, contradicting the docs

- `channel-bot-setup.tsx:354` renders `{onContinue && fullPage ? <Link to="/channel-bots/$botId">Open channel bot</Link> : null}`. The embedded content passes both props, so the dialog offers a router link that leaves chat. If clicked before the secret is copied, that also loses it.
- The docs (`CHANNEL_BOT_RELAY.md`, `05-frontend-ui.md`) say the dialog "does not ... navigate to ... bot management".
- The test misses this. It mocks `ChannelBotSetup` and asserts `role=button`, but the real control is a link.
- **Fix:** Add an embedded/stay-in-chat prop to `ChannelBotSetup` that suppresses the link, and add an unmocked test.

### M2. No connector-like completion or resume for Agent Key creation

- `/keys?tab=nyxid&action=create-key` (emitted by NyxBot `settings_link create_agent_key`, `assistant_team_tools.rs:122`) now opens `ApiKeyCreateDialog` in place. That is good. However, nothing safe is returned to the conversation, and the thread does not resume.
- Connect links resume through server watches. Here, NyxBot and the specialist never learn the key exists. This does not meet "safe completion results back to the conversation".
- **Fix, minimum acceptable:** After a successful create, offer a one-click, user-sent summary or a server watch that includes only non-secret metadata (key name, id, owner, scopes). Never include `full_key`. Otherwise, document this as an explicit scope limitation that the product owner accepts.

### M3. Broad platform-action coverage is partial

Opened in place:

- account panels
- service connect links
- hosted bot links
- `/channel-bots/connect/{platform}`
- Agent Key creation

Still opens a new Studio tab or leaves chat:

- `TelegramSetup` (`/channel-bots?connect=telegram-new&request_id=`, emitted by `telegram_new_status.rs`; this is a core bot-setup journey)
- `AgentDetails` (`/assistant?agent=&details=`), Automations, Machines, Saved logins
- `/keys?tab=services|pools`, `/channel-bots`, `/nodes`, `/approvals/*`, `/orgs`, `/triggers`, `/developer/apps`, `/settings/devices/onboard`, `/ai-setup`

Either extend the in-place coverage, starting with TelegramSetup and the assistant workspace pages, or state the deliberate scope in `05-frontend-ui.md` so the requirement is not overstated.

### M4. Secret screening is far narrower than documented

- `findSecretPath` matches only `Bearer …` and `nyx(id)?_…` (`schemas/assistant-actions.ts:76-77`).
- Telegram BotFather tokens (`\d+:[A-Za-z0-9_-]{30,}`), the canonical bot-setup secret, pass the check. So do `sk-…`, `ghp_`/`github_pat_`, `xox[abposr]-…`, JWTs and PEM blocks.
- A task like "create a bot with token 123456:AA…" renders active and, on click, copies the token into a NyxBot user message, its transcript and the model input. NyxBot's norm is to collect secrets on setup pages, not in chat.
- The docs claim "secret-shaped values cannot produce an active handoff".
- **Fix:** Add the common credential shapes, at parity with the backend `assistant_skill_authoring::credential_shaped`, and test each one. Otherwise, weaken the doc claim. The first option is preferred.

### M5. The capability header needs a CORS allow-list entry [in-flight change]

- The in-flight transport sends `X-NyxID-Chat-Capabilities` on every turn. It is not in the private CORS `AllowHeaders` list (`main.rs:2003-2019`), which already lists `X-NyxID-Client` for cross-origin clients.
- Same-origin `/api` deployments work. Any cross-origin API deployment would fail preflight for **every** NyxAgent turn.
- **Fix:** Add the header to the list (and to the CORS test), or confirm and document that deployment is strictly same-origin.

---

## Minor (optional)

- **m1.** In the group page (`NyxAgentGroupPage`, rendered outside `NyxBotHandoffContext`), `AssistantLink` drops `target=_blank` for `/assistant?c=nyxa-…` links but nothing intercepts them. The result is a same-tab full reload. Either keep `_blank` when there is no controller, or route through the router.
- **m2.** The handoff uses the transport's shared `"draft"` live key. If a NyxBot or specialist draft send is still unadopted, the click fails with "A turn is already active." While the handoff is pending, the New chat draft view shows the handoff's optimistic message. Consider a dedicated live key.
- **m3.** Several valid artifacts in one message each start a separate NyxBot thread. Activate only the first.
- **m4.** Credits failures in the handoff thread skip `notifyTurnCredits`, because `liveSend`'s `onFailed` is not passed.
- **m5.** The card copy should say plainly that the visible text will be sent **as your message** to NyxBot. A stronger optional alternative is to prefill an editable NyxBot composer and require the user to press Send.
- **m6.** Mobile (`react-native-markdown-display`) and `nyxid nyxbot` CLI transcripts of web-produced replies show the raw JSON fence. With the in-flight capability gate these clients are not offered the guidance, but history still renders raw. Consider an inert "Continue in the web app" rendering on mobile.
- **m7.** The `NyxBotHandoffContext` value is a new inline object on every page render, so every card re-renders. Memoize it.

## What I verified as correct

- **Link classifier:**
  - same-origin only, with no userinfo or fragment;
  - the bot token regex is anchored, takes no query, and allows no extra path segments;
  - Agent Key matching is exact (`size === 2`, so duplicate or extra params are rejected);
  - conversation links require the strict `nyxa-[a-f0-9]{32}` and a single parameter;
  - modified clicks keep browser behaviour.
- **Parser and renderer:** strict zod (version literal, bounded title and task, `.strict()` refusing extra authority keys), an 8 KiB input bound, and text-only rendering (no XSS surface). Streaming stays inert.
- **Click handling:** duplicate clicks within one mount are blocked (a ref plus `disabled`). A rejected send allows retry. An admitted send without an id is not replayed in the same mount.
- **Thread adoption:** it respects user navigation (`selection.current === source`). A transport identity change stops adoption after an account switch.
- **Hosted bot content:** reuses token-bound server checks (`useChannelConnectLink(token, user.id)`, keyed by user and link). Embedded mode follows no callback and does not redirect to login.
- **Turn admission:** handoff turns go through ordinary first-party human admission. No agent, conversation, or grant field is inherited from the specialist.

## Test evidence and limitations

- **Focused Vitest on the current (moving) tree:** 5 files, 27 passed and 2 failed. The failures are `nyxbot-handoff.test.ts` expectations broken by the parent's in-flight `useConfiguredProfile` edit. Results on a tree that is still changing are only indicative.
- **Tests do not cover the risks above:**
  - The card tests never remount after adoption, reload, or virtualize.
  - The hosted-bot unit test mocks `ChannelBotSetup`, so the secret, the link and dismissal are untested.
  - The Playwright hosted-bot test closes with X only.
  - The backend test asserts only presence or absence, not fingerprint stability.
- **Rust tests were not run by me** because the tree was being edited. Cargo is available on this machine via `export PATH="$HOME/.rustup/toolchains/1.98.1-aarch64-apple-darwin/bin:$PATH"`. Then run `RUST_MIN_STACK=8388608 cargo test --bins -- assistant_nyxagent::tests::handoff` (or a similar filter). The backend is a binary crate, so `--lib` finds nothing.
- I did not drive the real app in a browser or simulator.

## Re-review checklist

1. **B1:** The stable prompt is identical across turn origins on an eligible row (test). The capability is a turn-context fact. The instruction lives in stable text gated by thread-stable properties.
2. **B2:** The POST body has `agent_id = NyxBot` and no `model`. The thread model equals NyxBot's configured `agent.model`.
3. **B3:** A remount after adoption shows **Open NyxBot chat**. Reload persists. State is scoped per user. Ideally the server deduplicates.
4. **B4:** The gate uses server `handoff_eligible`, reply `origin === "user"`, no `via`, `complete`, and first artifact only. Legacy rows with no origin stay inert.
5. **B5/B6/M1:** Theme state survives the dialog. The secret cannot be dismissed accidentally. There is no bot-management link in embedded mode. All three are tested with the real `ChannelBotSetup`.
6. **M2–M5:** Each is fixed, or explicitly accepted and documented.
7. Rust and frontend suites pass on the final tree.

---

## Second pass: 2026-11-25

- **Verdict: CHANGES REQUESTED.** Two narrow items remain, plus the pending Rust evidence. Every first-pass blocker and major is resolved in code.
- **Snapshot reviewed:** the parent reports implementation SHA-256 `20d005a138e59c93c50cc1c62ca6d027e3df841f5892eebfbe2772a166242308`. That covers 49 files under the parent's procedure, excludes the review and verification records, and includes the Telegram request-recovery guard. I recorded my own digest of the same 49 files at 15:04Z, giving `e3bc4c0a5b9b11c5949861af88e4dc57c48b51e327acd47a2c796dd9fa261b9a`. My procedure: `{ git diff --name-only; git ls-files --others --exclude-standard; }` minus the two records, then `sort -u`, then `shasum -a 256` of each file, then `shasum -a 256` of that list. The two procedures differ, so the digests are not expected to match.
- **Evidence I ran:** 11 focused Vitest files, 373/373 passed. I did not run Playwright, and I started no backend compile so the parent's single cargo session had the target to itself. The parent reports 428 Vitest tests across 16 files, four Playwright scenarios, a production build, and lint, typography and i18n passing. Those are their results; I did not re-run them.

### Remaining items, required for approval

**R1. A pending handoff can appear as a phantom NyxBot thread. This is a regression from `clientDraftKey`.**
- **Where:** `frontend/src/lib/assistant/nyxagent-transport.ts:399-400`. `list()` merges every live entry whose key is not `"draft"` into the conversation index. A handoff waiting for adoption lives under `handoff:<sha256>`, and its `turn.conversation` is `{ id: "handoff:…", agent: NyxBot, title: <task> }`.
- **Repro:** Click **Continue with NyxBot**. While the POST is still waiting for response headers, trigger a sidebar `nyxAgentTransport.list(nyxbotId)` refetch. Any of these will do: a live event from another thread, a window-focus refetch, or a capacity wait in `direct_chat_limiter.admit` lasting up to the 45-second subscription deadline.
- **Impact:** A NyxBot thread with id `handoff:<hash>` appears in the sidebar. Clicking it routes to `?c=handoff:…`, because the search schema accepts any string. The row stays until the next NyxBot list refetch.
- **Fix:** Replace `id !== "draft"` with `isNyxAgentConversationId(id)`. Add a transport test that calls `list(nyxbotId)` during a pending `clientDraftKey` send.

**R2. The backend exclusion test is tautological.**
- **Where:** In `assistant_nyxagent_tests.rs::handoff_cards_are_offered_only_in_direct_specialist_user_turns`, these cases all assert `!specialist_handoff_capability(&row).contains("language nyxbot-handoff")`: the Orchestrator, Event, Channel, Group and Trigger origins, `report_to`, `guest_turn`, `group_id`, and the Orchestrator role.
- **Why it proves nothing:** The function returns only `HANDOFF_UI_NOTE` or `""`, so these nine assertions cannot fail.
- **Status of the code:** I traced it and it is correct today. Each condition returns `""`.
- **Fix:** Assert `.is_empty()`, as the voice case already does. Optionally, also assert that `turn_notes` strips the raw `turn.note` marker and adds the capability fact exactly once.

**Rust evidence.** The filtered handoff and CORS tests must actually pass on the final tree. When I wrote this, they were still compiling in the parent's session.

### First-pass findings: re-verified against real code

- **B1: resolved.**
  - `private_handoff_thread` gates the stable instructions only on thread-lifetime properties.
  - The per-turn capability fact is added to turn context by `specialist_handoff_capability`. That function requires every one of: eligibility, a User origin, no `reply_channel`, `report_to`, `with_nyxbot`, voice, invocation, trigger, platform/adapter origin, `asked_from` or `also_deliver`, and a first-party, header-derived `HANDOFF_UI_NOTE`.
  - The raw note is stripped from turn notes.
  - `base_prompt` is byte-identical across origins and with or without the capability, as the test asserts.
  - Mobile and CLI clients no longer advertise the capability.
- **B2: resolved.** The handoff passes the live NyxBot descriptor (`kind === "nyxbot"`, not destroyed) and no `model`. The server then uses `agent.model`, because the default-model branch requires `agent_id` to be absent (`services/assistant_nyxagent.rs:2327`). The button stays disabled until the roster has loaded.
- **B3: resolved, with residuals.**
  - Receipts are keyed by a SHA-256 of `(actor, source conversation, server message id, parsed proposal)`.
  - Cards activate only on settled history rows carrying `origin`, so the id is the stable server id.
  - Started receipts reopen the same thread.
  - The behaviour fails closed in every uncertain case. A pending receipt restored after reload becomes "accepted". A send admitted without an id becomes "accepted". Only a pre-admission rejection clears the receipt and allows a retry.
  - Memory resets on account change. Cross-tab storage events reload state. A Web Lock serializes the same card.
- **B4: resolved.**
  - The gate requires all of: server `handoff_eligible`, a specialist that is not destroyed, the NyxBot roster loaded, `status === "complete"`, `origin === "user"` (verified that `AssistantMessage.origin` is persisted on reply rows and serialized lowercase), and no `via`.
  - Only the first top-level fence is marked, via a sanitized `dataHandoffFirst` hProperty.
  - Raw HTML stays literalized, so the model cannot forge the mark.
- **B5: resolved.** Only the standalone shell calls `useApplyTheme`. A real-DOM test checks that theme state survives.
- **B6: resolved.** The hosted bot dialog blocks Escape and outside-click dismissal. An e2e test with the real `ChannelBotSetup` checks that the one-time secret survives.
- **M1: resolved.**
  - The embedded flow passes `stayInPlace`, and the "Open channel bot" link is hidden.
  - The Telegram request id now prefers the hosted link's `telegramRequestId`.
- **M2: resolved.** `completePlatformSetup` accepts a strict `kind` plus a UUID.
  - It is fenced on identity revision and user id before, during and after reading the resource back through `authorityGuard`.
  - It sends exactly one user continuation, containing the resource id only, to the source conversation captured when the dialog opened (`adoptExisting`). It resolves on admission.
  - On a rejected send, the dialog stays open with the result and allows a retry. Double clicks are guarded.
  - Creation hooks fence actor changes through `authorityGuard`.
  - Hosted service and bot links keep their server watch and send no client continuation.
  - Exact Telegram request recovery (`target.requestId`) sends no continuation, per the producer contract: its notice URLs are not bound to any source conversation, and hosted requests are owned by the server.
- **M3: resolved as documented scope.**
  - Telegram request recovery opens a modal.
  - Agent details opens the current-chat sheet.
  - Automations, Machines and Saved logins, plus the listed Studio entry points, navigate in the same app.
  - The docs distinguish in-place dialogs from full app pages.
- **M4: resolved.** Common credential shapes are refused and tested, and the doc claim is now limited to "known shapes".
- **M5: resolved.** The header is in the CORS allow-list, with a test.
- **Minor items.** The shared draft key is fixed by `clientDraftKey`, which introduced R1. The card copy now says the visible task is sent as your message. Credits notification is not added to handoff sends and remains minor.

### Residual limits I accept (non-blocking)

1. **Receipts are browser-local and bounded to 200 entries.** They are not a server or cross-device exactly-once guarantee. Evicted or other-device cards can be sent again. An "accepted" receipt (from an interrupted send or a reload during a pending send) disables the card permanently, and the user must copy the task manually. Receipts, which are hashes plus `nyxa-` ids, survive logout on a shared browser.
2. **Continuations may be duplicated or lost at the network edge.** A pre-header network failure after server admission is treated as a rejection, the same semantics as the composer, so a retry can duplicate the continuation. An account switch mid-POST discards the response, so a created Agent Key's one-time secret can be lost and the key must be revoked. That is fail-closed by design.
3. **Telegram status anchors leave chat.** `telegram-new.tsx:238/241` "Check status" and "Start a new setup" are plain absolute anchors inside the portal and are not intercepted. Clicking one navigates the whole tab out of chat.
4. **The direct (non-hosted) channel-bot dialog can still lose its secret.** That dialog, which the backend no longer emits (legacy transcripts only), has no Escape guard while showing a webhook secret. This predates this change.
5. **Embedded Telegram setup can briefly show a stale Decline button.** Embedded `stayInPlace` skips `link.refresh()` when a Telegram request is saved. The hosted status poll corrects it, but **Decline request** can be visible for a short time.
6. **Workspace links now open in the same tab.** Links to Studio pages navigate in the same tab instead of a new one; modified clicks keep new-tab behaviour.
7. **Raw artifacts on other clients.** Native mobile and the CLI show historical browser artifacts as an inert fenced code block.
8. **Not exercised.** I did not drive the live app or the backend end-to-end. The Rust results are pending.

### Path to APPROVED

Fix R1 with a test and R2. Report the filtered Rust tests (handoff and CORS) passing and the new snapshot digest. A third pass will then check only those deltas.

---

## Third pass (R1/R2 deltas): 2026-11-25

- **Verdict: no remaining code findings. Final APPROVED is withheld until the parent reports final-tree Rust results** for the filtered handoff and CORS tests. The handoff test passed on an earlier build, but its assertions changed while that build compiled.
- **Snapshot:** the parent reports `7f838c6dd0859f59d1c847e675ac7f55e0b2d9f54ad5fb70e68a515997b0d5b0` (49 files, parent procedure). My independent digest of the same 49 files at 15:06Z is `93145854b3662f30fe9f51d7ab3b3b14d748f62009497bcd474823464dcddb94`, using the second-pass procedure.

### R1: resolved

- `nyxagent-transport.ts:400` now merges a live turn into `list()` only when `isNyxAgentConversationId(id)` is true.
- Ordinary sends on existing threads use the real `nyxa-` id, so they still appear. The `"draft"` and `handoff:<sha256>` keys are excluded.
- The new transport test holds a `clientDraftKey` handoff POST before its headers arrive. It asserts that the scoped `list(nyxbotId)`, the stored scoped conversations and the full `list()` all contain only the real saved thread. Against the previous `id !== "draft"` check this test would fail, so it is a real regression guard.
- A companion test checks that the handoff body is exactly `{ agent_id, text }`, with no model, and that it carries the capability header.
- I ran `nyxagent-transport.test.ts` together with the handoff card and helper suites: 186/186 passed.

### R2: resolved

- Every exclusion check (the five non-user origins, `report_to`, `guest_turn`, `group_id` and the Orchestrator role) now asserts `specialist_handoff_capability(&row).is_empty()`.
- The row keeps `note = HANDOFF_UI_NOTE` throughout, and a positive control first confirms the capability is emitted. So each exclusion now fails if its condition is removed.

### Remaining condition for APPROVED

The filtered Rust handoff and CORS tests must pass on the final tree (digest `7f838c6d…` under the parent's procedure), with no source changes after that run. The residual limits in the second pass still apply.

---

## Final verdict: 2026-11-25

- **Verdict: APPROVED.** This approves the implementation snapshot `7f838c6dd0859f59d1c847e675ac7f55e0b2d9f54ad5fb70e68a515997b0d5b0` (parent procedure, 49 files, review and verification records excluded).
- **No changes since the third pass:** at 15:14Z I recomputed my independent digest of the same 49 files and got `93145854b3662f30fe9f51d7ab3b3b14d748f62009497bcd474823464dcddb94`. That is identical to the third-pass value, so the tree I reviewed has not changed. `git diff --check` is clean.
- **Rust results:** reported by the parent, using the final assertions on this tree. I did not run them myself, so that no second cargo build would compete for the same target.
  - Environment: `env PATH=$HOME/.rustup/toolchains/1.98.1-aarch64-apple-darwin/bin:$PATH RUST_MIN_STACK=8388608 CARGO_BUILD_JOBS=4`
  - `cargo test -p nyxid --bin nyxid-server handoff_cards_are_offered_only_in_direct_specialist_user_turns -- --nocapture`: 1/1 passed, 12,527 filtered out, exit 0. This came after a full rebuild of the final assertions (6m55s).
  - `cargo test -p nyxid --bin nyxid-server private_cors_allows_analytics_narrowing_header_only_for_trusted_origin -- --nocapture`: 1/1 passed, exit 0, on the reused build.
  - Both runs printed the existing non-fatal linker unwind-size and dependency future-compatibility warnings.
- **Frontend evidence:**
  - Mine: 11 focused files, 373/373 passed (second pass), and transport, card and helper suites, 186/186 passed (third pass).
  - Reported by the parent: 438/438 across 14 focused files, 171/171 for the transport delta, four Playwright scenarios, `tsc` and targeted ESLint passing, and a production build with lint, typography and i18n passing.
- **Approval scope:** all first-pass blockers and majors and both second-pass items are resolved in code and covered by tests. The residual limits in the second pass are accepted and not blocking:
  - receipts are browser-local and bounded;
  - admission at the network edge has the same semantics as the composer;
  - an Agent Key secret is lost if the account switches mid-POST;
  - Telegram status anchors inside the portal navigate the tab;
  - the legacy direct bot dialog has no Escape guard;
  - the Decline button can be briefly stale in embedded Telegram setup;
  - workspace links open in the same tab;
  - mobile and CLI show the artifact as raw text;
  - neither the live app nor the backend was driven end-to-end.
- This approval covers code review only. It does not authorize merging, deploying, or changing rules or protections. Any source change after this snapshot needs a new review of that delta.

---

## Integration delta review: 2026-11-26

- **Verdict: APPROVED.** This approves the integration of the feature into current `dev`. I found no defects in the conflict resolution, in the auto-merges, or in how upstream changes interact with eligibility, model selection, admission or completion.
- **Condition:** the PR's backend CI must pass (compile, Rust tests including the two filtered handoff and CORS tests, CodeQL). The parent ran `cargo fmt` only, so no Rust build has run on this merged tree. Per instructions, I started no cargo or build process.
- **Reviewed:** merge commit `241da803e727ad26dc3c6444aad0282b78be1741`, with parents feature `508195a20` and `origin/dev` `fbd9b70b8`. The working tree was clean. I did not edit any source.

### Method

1. **The feature commit is the approved code.** For the 49 implementation files, I hashed each file's content in `508195a20` with the second- and third-pass procedure (record files excluded) and got `93145854b3662f30fe9f51d7ab3b3b14d748f62009497bcd474823464dcddb94`. That is identical to the approved snapshot.
2. **Feature delta versus PR delta.** For every file, I compared the `-U0` delta from `575c5f596` to `508195a20` with the delta from `origin/dev` to `241da803e`. 43 of the 49 deltas are identical. The six that differ are:
   - the two backend files;
   - `chat-message.tsx`;
   - the three legacy i18n monoliths, which dev deleted and replaced with nine split catalogs.
3. **Upstream changes on our dependencies.** I read every upstream change from `575c5f596` to `origin/dev` that touches a dependency of the feature:
   - models;
   - the turns handler, `TurnStart`, `turn_notes` and the restart enrollment path;
   - the transport, the NyxAgent hook, `api-client`, the text-block remark pipeline and the chat page;
   - channel setup, channel hooks and the hosted-link hook;
   - connect-link content, the NyxBot roster and the auth store.

### Conflicts and non-identical deltas

- **`chat-message.tsx` (the real conflict): correct.**
  - dev's `contentOverride ?? <TextBlock …>` fallback is kept, and the TextBlock keeps every handoff gate unchanged: `eligible`, `role === "assistant"`, `status === "complete"`, `origin === "user"` and `!via`.
  - `contentOverride` has a single producer, the voice message group renderer (`assistant-chat-page.tsx:1323`). Voice-grouped replies therefore render inert. That matches the backend, which never advertises the capability on voice turns (`voice_request_id`).
- **Backend: formatting only.**
  - `cargo fmt` reflowed the header-capability check (`handlers/assistant_nyxagent.rs`) and the `turn.note` predicate (`services/assistant_nyxagent.rs`). The tokens are identical.
  - `ConversationResponse` is still built only through `From<AssistantConversation>`, which sets `handoff_eligible`.
- **i18n: correct.**
  - All nine feature keys landed in the split catalogs (`app/assistant`, `app/shared`, `app/unattributed`), with en, zh-CN and zh-TW values byte-identical to the approved monolith delta.
  - The monoliths are absent on both `dev` and HEAD.
  - *Optional nit:* two superseded strings are not referenced by any code and could be dropped later: "NyxBot will continue this task in a new chat…" and "NyxBot accepted this task…".

### Upstream interactions checked

**Backend admission and capability**
- `start.note = HANDOFF_UI_NOTE` is still set only for first-party humans that send the header.
- `TurnStart::from` still sets `note: None`.
- No new upstream writer overwrites the note on browser turns. `platform_personal_turn_service` and `collaboration_group_service` set notes only on their own channel or group starts.
- `turn_notes` still adds `specialist_handoff_capability` and strips the raw marker. No other code renders `turn.note`.
- The new restart enrollment clones `start` into `admitted` (`handlers/assistant_nyxagent.rs:2698`), so restarted turns keep the note. If a restart path ever dropped it, the result would fail closed: no capability, so no card.

**Eligibility**
- Upstream added five `AssistantConversation` fields: `meta_grant`, `org_use_revision`, `reply_channel_meta_grant`, `stopped_window` and `thread_secret_fence`.
- It added five `ActiveTurn` fields, all for restart, reattach or settlement: `execution_attempt`, `group_reply_in_settlement`, `restart_admission`, `restart_eligibility` and `restart_offer`.
- None of these introduces a new thread kind that `private_handoff_thread` would have to exclude.
- The org specialist maintainer and user lists (#2977, #2980) apply only to org specialists. Those threads carry `agent_owner_id`, so they are already ineligible.
- dev has no Health or isolated specialist kind. `AgentKind` is still `Nyxbot | Specialist`.

**Model (B2)**
- `begin_turn` still resolves `start.model.unwrap_or(agent.model)` (`services/assistant_nyxagent.rs:2674`).
- The handler's orchestrator-default branch still requires `agent_id` to be absent.
- At HEAD, the transport still omits `model` when an agent is passed without a model, and still sends the capability header.
- `nyxBotOf` still returns the roster entry with `kind === "nyxbot"`.

**Transport, R1 and drafts**
- HEAD keeps the `isNyxAgentConversationId(id)` gate in `list()`, the `clientDraftKey` isolation and adoption unchanged.
- Upstream's `forgetAgent` now aborts live turns, but only for indexed threads of the forgotten agent. A pending NyxBot handoff is never indexed under that agent, so it is unaffected.

**Rendering**
- dev added `remarkAutolinkPunctuation` before `remarkSafeModelContent`. It rewrites only `link` nodes, so fenced code and the `dataHandoffFirst` marker are untouched.

**Chat page**
- dev added a `restricted` state for `can_use === false` and for history 403/404. It renders a minimal shell with no message list, so no handoff provider is mounted.
- Navigating away clears the selection, so adoption navigation (`selection.current === source`) cannot pull the user back.

**Completion and setup**
- dev added `MetaCustomerSetup` to the `ChannelBotSetup` completion view. It has no links and does not navigate, and the hosted dialog's Escape and outside-click guards still cover it.
- The hosted-link `refresh` type change is compatible with our callers.
- dev's connect-link content also suppresses callbacks when embedded, which matches our contract.
- The `authorityGuard` hooks merged cleanly: `use-channel-bots` and `use-api-keys` received only unrelated upstream edits.

### Evidence and residuals

- **Parent-reported:** 483/483 focused Vitest across 17 files, 4/4 Playwright specialist-handoff scenarios, a production build, and full frontend lint, typography and i18n (0 errors, 33 existing warnings). `cargo fmt` changed only the two expressions above. I did not re-run these.
- **Unchanged residuals:** the residual limits from the second pass still apply.
- **One integration-specific residual:** a voice reply that is *not* grouped would still pass through the gated TextBlock if it carried `origin === "user"`. The backend never offers the capability on voice turns, so this is a defence-in-depth gap only, not a defect.

### Addendum: CI follow-up commit `24eb67532` (2026-11-26)

- **Verdict: APPROVED is unchanged.** This commit contains no changes to implementation behaviour.
- **`frontend/src/pages/channel-bots.test.tsx`: test-only.**
  - Nine `api.post` call assertions across twelve cases now expect a third argument, `{ authorityGuard: expect.any(Function) }`. Their exact payloads are kept.
  - This catches the tests up with the already-approved `useCreateChannelBot(authorityGuard)` path. `ChannelBotSetup` always supplies the identity-revision fence, including on Studio pages, and the hook passes it to `api.post`.
  - I ran this file locally: 54/54 passed.
- **CLI wizard bundle: generated output.**
  - The minified `cli/src/wizard/assets/index.html` changes `post(e,t){…{method,body}}` to `post(e,t,n){…{...n,method,body}}`. That is exactly the approved `api-client.ts` change; `api-client.ts` is in the wizard manifest closure.
  - The only other change is one regenerated CSS utility rule.
  - `bundle-meta/index.hash` was updated to match. CI's wizard freshness check is the authority for this file.
- **Evidence condition, refined:** the parent reports that CI's dev profile runs only `cargo clippy` (binary and lib) and `cargo fmt`, so it gives compile evidence but runs no backend tests.
  - The approval therefore requires two things on the final head: CI is green, and the parent's local run of the two filtered Rust tests (`handoff_cards_are_offered_only_in_direct_specialist_user_turns` and `private_cors_allows_analytics_narrowing_header_only_for_trusted_origin`) passes.
  - Backend sources are unchanged since `241da803e`.

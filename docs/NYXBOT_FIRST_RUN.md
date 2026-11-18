# NyxBot first-run onboarding

New NyxBot users get a short, skippable intro on web (`/assistant`) and mobile
(the NyxBot home). A first-timer is greeted by name on a welcome page first;
a replay starts at the questions. It then has three parts:

1. **Who it's for and interests.** The user first picks who NyxBot is for: just
   me, my team, or my business. The topic list then follows that answer, using
   each topic's `work_styles`. Personal and team topics cover getting work done
   (email, calendar, research, code, docs, team chat, reports). Business topics
   are SaaS-like (customer support, sales and leads, marketing, reports,
   payments). Switching the answer drops picks that no longer apply, and only
   the pills that differ animate: leaving pills fade out in place, shared
   pills glide to their new places, and new pills fade in one after another.
   Every pick is optional.
2. **Three intro screens.**
   - **Chief of staff, specialists and "You stay in control"** play live,
     scripted replays on web (`live-replay.tsx`). They use the real chat
     message bubbles and action card, driven by a timeline of sample data, so
     they follow the user's theme, text size and density and never touch real
     conversations. The chief-of-staff example follows the first topic the
     user picked. With reduced motion they show the finished conversation.
     Mobile plays GIF recordings of the real assistant from
     `mobile/assets/onboarding/`. They are captured in dev mock mode, so they
     contain fixture data only; regenerate them with
     `frontend/scripts/record-onboarding-clips.mjs`. The script turns on two
     dev-only fixture switches: `partialText` (no placeholder stream) and
     `markContinuations` (decision continuations render as system lines, as
     on the server). Neither changes the e2e defaults.
   - **You stay in control** shows an action card being allowed. On web an
     access-mode explainer follows it.
   - **Specialists copy:** specialists are presented as people who are great
     at one thing. The owner gives each a fixed role and scope, and works with
     them directly or through NyxBot.
3. **Next steps and the tour.** One-click cards ranked by the answers. Clicking
   a connection card opens live provider choices and hosted setup first (below);
   other cards send their prompt to NyxBot as the user's message. NyxBot then
   does the work with its normal tools and confirmations. The same
   cards stay in **Suggested for you** on the NyxBot home until each step is
   done. The section's **Hide** hides them all until the intro is retaken;
   cards have no individual dismiss. On web, **Show me around** starts a
   spotlight tour of the live screen. The tour visits the composer, the
   suggestions, the agents, **Create new agent**, Approvals and Automations.
   Targets are found by their `data-tour` attributes, and steps whose target
   is not on screen are skipped.

## Storage and API

- **Collection:** `assistant_onboarding`, with `_id` = user ID. It stores the
  intro version, the outcome (`completed` or `skipped`), the topics, the work
  style, when the suggestions were hidden, and timestamps. The answer
  enums are closed; values written by a newer replica deserialize as
  `unsupported` and are ignored. Personal erasure removes the row.
- **`GET /api/v1/assistant/nyxagent/onboarding`** returns:
  - `show_intro`;
  - the stored `profile`;
  - the `topics` and `work_styles` choices with their labels;
  - up to `limit` (default 3, maximum 6) `recommendations`;
  - `suggestions_hidden`, which tells the home to hide its section. The intro
    still shows the recommendations.
- **`PUT /api/v1/assistant/nyxagent/onboarding`** with
  `{outcome, topics, work_style}` replaces the answers. It writes the
  metadata-only audit event `assistant_onboarding_saved`. Saving also clears a
  hidden-suggestions mark.
- **`POST /api/v1/assistant/nyxagent/onboarding/suggestions/hide`** hides the
  home's suggestions and returns the refreshed state.
- **Access:** all onboarding routes are human-only (first-party sessions) and
  follow the NyxAgent engine flag, like the other `/assistant/nyxagent` routes.

## When the intro shows

`show_intro` is true only when both of these hold:

- the user has no onboarding row;
- the user has no `assistant_conversations` row.

Existing NyxBot users are therefore never interrupted, and no backfill is
needed. Anyone can replay the intro: on web from the account menu (**Replay
intro**) or the home section, on mobile from the home section.

## What NyxBot sees

Saving the answers writes one NyxBot memory note with the fixed ID
`nyxid-onboarding-interests`. The note is built from the closed label set
only, never free text. If the user gives no answers, the note is removed.

Memory notes are already part of fresh turn input, so this adds no database
read to turns. The user can see and delete the note under **What NyxBot
remembers**. If memory is full (50 notes), the write is skipped.

## Recommendations

Recommendations come from a fixed catalog in
`services/assistant_onboarding_service.rs`. Each entry has a base priority.
It gains 40 points for each matching topic and 20 for a matching work style.

An entry is left out once it is done. Done means one of:

- a matching active service slug;
- a live specialist whose name matches;
- any schedule;
- an active NyxBot channel.

The checks run four small queries in parallel, and only when the
recommendations are requested.

## Telemetry

Funnel events reuse the consent-gated dialog events
(`ui.dialog_*` / `ui.mobile_dialog_*`) with `dialog_id: "nyxbot_intro"`; the
web tour uses `dialog_id: "nyxbot_tour"`.

### Connection suggestions

New clients treat `RecommendationResponse.connect: true` as a provider-choice
setup before starting the chat. On selection, the human-only
`GET /assistant/nyxagent/onboarding/suggestions/{id}` reads current catalog and
personal connection state. It returns `connected`, `choices` (catalog slug,
name and capability-specific OAuth scopes), and a follow-up `prompt`. This read
never mints a token, connects an account or changes agent grants.

If the account is not connected and no provider choices are available, web and
mobile close the chooser and start the chat with the suggestion's original
prompt. NyxBot can guide the person through setting up their own provider app.
This also applies when the last provider disappears on a later recheck; it
never mints a link or substitutes the connected-account follow-up prompt.
The onboarding list itself adds no per-person provider-availability reads.

Calendar offers Google Calendar and Microsoft 365 only when their live public
catalog service and hosted OAuth configuration are usable. An unconfigured
Microsoft Entra app is not offered as a working calendar connection. Inbox,
documents, GitHub, X, team chat and Stripe suggestions use the same flow. OAuth
requires usable configured client credentials (platform or this person's own);
API-key services use the existing secure hosted credential form. A selected
provider calls the existing `POST /connect-links` and opens hosted setup; no
provider button merely inserts chat text. Microsoft calendar links explicitly
request `Calendars.ReadWrite`; inbox links request mail read/write/send scopes.

The chooser refreshes before minting a link and after setup. Only an active
same-person credential counts as connected; a provisioned `pending_auth` row
does not. A connected account skips the link and starts NyxBot with the setup's
follow-up prompt to use that account. Native embeds the existing hosted form
inside the chooser, including during the first-run intro, and continues after
verified completion. If setup opens in an external browser, Continue rechecks
the connection on return. Other suggestions retain the normal conversation
flow. Older clients/servers keep the original prompt-only behavior.

Web chooser controls, status, errors and accessible labels follow the active
Studio/NyxBot language (English, Simplified Chinese or Traditional Chinese).
Provider names and the original suggestion prompt are retained as authored.

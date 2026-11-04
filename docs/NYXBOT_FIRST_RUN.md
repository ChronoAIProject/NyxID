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
   - **Chief of staff, specialists and "You stay in control"** play recorded
     replays of the real assistant. The replays are captured in dev mock mode, so they contain
     fixture data only. Web uses VP9 WebM with an H.264 MP4 fallback and a poster
     frame, imported from `frontend/src/assets/onboarding/` so the files are
     content-hashed;
     mobile uses GIFs in `mobile/assets/onboarding/`. To regenerate them after
     a UI change, run `frontend/scripts/record-onboarding-clips.mjs`. It turns
     on two dev-only fixture switches: `partialText` (no placeholder stream)
     and `markContinuations` (decision continuations render as system lines,
     as on the server). Neither changes the e2e defaults.
   - **You stay in control** replays an action card being allowed. On web an
     access-mode explainer follows it.
   - **Specialists copy:** specialists are presented as people who are great
     at one thing. The owner gives each a fixed role and scope, and works with
     them directly or through NyxBot.
3. **Next steps and the tour.** One-click cards ranked by the answers. Clicking
   a card sends its prompt to NyxBot as the user's message. NyxBot then does
   the work with its normal tools, connect links and confirmations. The same
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
- **Access:** all three routes are human-only (first-party sessions) and
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

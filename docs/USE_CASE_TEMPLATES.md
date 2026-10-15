# Use-case onboarding skills

Status: repository authoring drafts and implementation contract. The three skills
are not installed, published to Ornn or attached to any agent. No setup tools,
provider adapters or runtime changes are implemented by this documentation.

## Authoring decision

Use a `SKILL.md` for each use case. The skill tells authenticated owner NyxBot
what to ask, which existing onboarding tools to call, which specialist to
create/reuse, and what evidence establishes completion. There is no separate
use-case YAML file or new YAML workflow interpreter.

The short name/description frontmatter is standard skill metadata. The onboarding
procedure, requirements, tool guidance and checks live in Markdown. Backend tool
schemas and setup records remain structured; prose never becomes permission or
proof that an integration works.

## Prepared skills

| Skill | Specialist and outcome |
| --- | --- |
| [Set up FAQ](skills/setup-restaurant-faq/SKILL.md) | FAQ specialist using reviewed facts over Telegram and inbound telephone |
| [Set up queue management](skills/setup-restaurant-queue/SKILL.md) | Private staff queue specialist with bounded entry-specific SMS alerts |
| [Set up reservations](skills/setup-restaurant-reservations/SKILL.md) | Reservation specialist taking Telegram/phone requests into Notion |

The [restaurant requirements](RESTAURANT_USE_CASES.md) describe contact identity,
channel sharing, Notion mapping and business acceptance in detail. Each skill is
self-contained so its future package does not depend on repository-relative files.

Several selected skills participate in one saved, interactive owner setup. The
[NyxBot onboarding session contract](NYXBOT_ONBOARDING_SESSIONS.md) defines its
conversation association, shared requirements, progress panel and card actions.

## How the skill reaches NyxBot

The public BotFather explains the use case and initiates identity handoff. After
signup/linking, the owner explicitly adopts the draft in authenticated NyxBot.
A use-case handoff must carry the selected use case/setup reference, not private
transcript or credentials. PR #1901's verified personal-manager route is intended
to provide access to the owner's existing NyxBot; it does not itself select or
attach these onboarding skills. At inspected head `f8f623a42`, that PR remains
open with runtime/continuation validation outstanding. Its personal route uses
owner turns, which include existing attached skill guidance.

Publish reviewed versions through the existing Ornn-compatible process. Owner
NyxBot searches/previews the skill and requests attachment to itself through the
existing permission mode: a card in Auto/Ask every time, or an automatic receipt
in Full access. The owner can also attach it in the UI. Do not silently install
it on account creation. Existing skill reads use that person's live Ornn access,
exact GUID/version/hash and dependency pins. No platform-master fallback or new
registry is introduced.

NyxBot reads the attached skill using `nyxid__skill_read` and guides setup in its
owner thread. Setup skills stay on NyxBot because specialists cannot create
agents, grant access, or use hosted/channel onboarding tools. Separate operating
skills are required for the initial use cases and receive their own reviewed pins.
Prepare owner-private copies of the selected defaults so the user can request
later improvements. These operating packages still need authoring/publication;
see [operating skill lifecycle](USE_CASE_SKILL_LIFECYCLE.md) for existing editing
support and required copy, provenance, testing and activation extensions.
Guest turns currently omit skills; attachment does not enable a public runtime.

## Use-case entry links and signup continuation

Proposed browser links on the configured frontend origin:

```text
/assistant?onboarding=restaurant-faq
/assistant?onboarding=restaurant-queue
/assistant?onboarding=restaurant-reservations
/assistant?onboarding=restaurant-faq,restaurant-reservations
/assistant?onboarding=restaurant-faq,restaurant-queue,restaurant-reservations
/assistant?onboarding=restaurant-suite
```

These URLs are proposed, not implemented. `restaurant-suite` selects all three
through a curated server mapping. Comma-separated IDs select a custom subset;
validate a bounded known set, canonicalize ordering and deduplicate IDs. Reject
duplicate query parameters, unknown IDs and mixed suite/individual syntax.
The parameter is a public selection hint, not
a skill URL, arbitrary prompt, package pin, user ID, credential or grant.

A "Sign me up for reservations" CTA can target:

```text
/register?return_to=%2Fassistant%3Fonboarding%3Drestaurant-reservations
```

Existing login/register/social/MFA handling preserves trusted `return_to` values;
reuse it and test every supported signup/verification path. Already signed-in
users go directly to the assistant entry. Invitation/account policy still applies.

The entry resolves the use case and shows its matching setup introduction
automatically: "Let's set up your reservation specialist. We'll connect Notion,
choose where requests are saved, and set up Telegram and telephone."
There is no need to make the owner retype their use case. A mere GET/navigation
does not attach a skill, start a billable turn, connect a provider or create an
agent. Use the explicit start/skill-review action for those transitions.

Implementation surface:

1. Extend `frontend/src/lib/assistant/search.ts` and the assistant page to accept
   the closed `onboarding` selection. Reject unknown values with an actionable
   message. Do not combine it with arbitrary `agent`, `g`, `c` or `setup` parameters to
   redirect setup into another context.
2. Add a versioned admin-managed catalog mapping each public use-case ID to its display
   copy, availability and exact published Ornn skill pins. Resolve visible pins
   under the acting person's existing Ornn access. No user-supplied package or
   registry URL is accepted. Missing/unpublished skills produce an explicit
   unavailable state; these repository drafts do not yet supply real pins.
   Admin publication does not attach a skill to users. Snapshot definition
   revisions at selection; updates require explicit migration/review. See the
   [admin catalog and carryover contract](NYXBOT_ONBOARDING_SESSIONS.md#admin-managed-use-case-catalog).
3. Add authenticated, first-party start/resume orchestration. A possible new API
   is `POST /api/v1/assistant/onboarding/start` with `use_case_ids: [...]` and an
   idempotency key; this route does not exist today. Derive the owner from the session, create
   or resume an owned setup/conversation, and return its state and pending review.
   Reuse the existing skill attachment card, exact pins and revision checks.
4. If the required skill is already attached at the reviewed version, the start
   action can begin the setup turn. Otherwise follow the effective mode: show
   owner review in Auto/Ask every time or an automatic receipt in Full access.
   The URL alone never approves skill installation or changes permission mode.
   Bind one durable start intent to the resulting turn so refresh, double-click,
   approval wake and reconnect cannot duplicate the first message or specialist.
5. Replace the selection URL with `/assistant?c=<conversation-id>&setup=<setup-id>`
   after resolution; authorize ownership and the setup/conversation association.
   Resume the active setup for repeated clicks; creating a second
   installation is an explicit action. On decline, expiry, unavailable skill or
   failed start, keep useful recovery state without replaying external effects.

For a public Telegram signup link that must also establish personal access,
retain PR #1901's server-issued linking intent and same-private-chat confirmation.
Store the validated use-case ID set beside that intent, then offer/start the owned
onboarding after its required review. Do not replace the linking proof with a
plain query parameter or assume web signup authenticates a Telegram sender.
Saved anonymous facts require explicit adoption; a public use-case slug alone
needs no private draft payload. Use one server-owned setup reference to coordinate
web and Telegram so both surfaces cannot independently start duplicate setups.

Acceptance: signed-in/signed-out entry; email/social/MFA/verification returns;
correct skill selection; existing/new skill attachment; expired/declined review;
refresh/double-click/parallel tabs; wrong-account intent; unknown selection;
missing Ornn visibility; successful post-approval start; and no automatic service
grants or agent creation from URL loading. Include the real connection-completion
resume in PR #1901 integration acceptance.

Render the selected cases as a persistent setup panel in the owner conversation.
Chat edits and card actions update the same validated state; connect completion
resumes the relevant step once. See the [interactive session contract](NYXBOT_ONBOARDING_SESSIONS.md)
for focus changes, shared resources, revisions, partial activation and Telegram.

## Standard structure for a new onboarding skill

Use this authoring outline, completing each section with concrete guidance:

```markdown
---
name: setup-business-use-case
description: Describe the owner setup request this skill handles and its outcome.
---

# Set up the named use case

## Outcome and execution context
Describe the resulting specialist, audience, channels and available/blocked parts.

## Collect and inspect
List required business details, existing resources to reuse, and capabilities
that must exist. Distinguish missing user configuration from missing product code.

## Guide connections
Name existing discovery/connect tools, returned human setup links, completion
signals, resource selections and how to recover from decline or expiry.

## Create and configure the specialist
Define its role, connection/operation limits, private versus customer context,
default operating skills, private customization and provisioning/reuse rules.

## Test, activate and report
Define authorized test effects, observable results, activation gates, durable
references, remaining blockers and the next human action.
```

Keep the skill outcome-specific. Use actual advertised tool schemas rather than
copying API payloads that will drift. No raw credentials, user IDs or live resource
IDs belong in the shared skill. Resolve those from the owner's current setup.

## Existing tools and their limits

| Step | Existing tool | Required interpretation |
| --- | --- | --- |
| Discover inventory | `nyx__list_connected_services`, `nyx__discover_services` | Existing but unavailable connections are not ready |
| Inspect operations | `nyx__search_tools` | Use returned names/schemas; generic provider APIs may lack the required business flow |
| Start service consent | `nyx__connect_service` | Omit credential; return the exact hosted link and request only supported scopes |
| Resume connection | `nyx__wait_for_connection` and existing connect-link watches | Verify live authorization after completion; do not poll forever or ask for secrets |
| Inspect specialists | `nyxid__list_subagents`, `nyxid__read_subagent` | Reuse only the selected compatible agent; use immutable IDs |
| Create specialist | `nyxid__spawn_subagent` | Create empty/private with no initial task; platform-reserved agents belong to their provisioner |
| Configure authority | `nyxid__grant_subagent`, `nyxid__set_agent_operations`, `nyxid__set_guest_access` | Existing separate operations; bounded onboarding needs coordinated reviewed provisioning |
| Prepare skills | `nyxid__search_agent_skills`, `nyxid__agent_skill_versions`, `nyxid__preview_agent_skill` | Resolve exact visible packages, not guessed names/versions |
| Attach skill | `nyxid__get_agent_skills`, `nyxid__set_agent_skills` | Preserve pins/revision; owner card in Auto/Ask every time, automatic receipt in Full access |
| Ordinary channel setup | `nyxid__channel_bot_setup_link`, `nyxid__connect_channel_bot`, `nyxid__link_channel_bot` | Personal flows may auto-link; not a bypass for platform BotFather installation fences |
| Delegate ready work | `nyxid__message_subagent` | Invoke only a prepared specialist with appropriate task authority |
| Open existing settings | `nyxid__settings_link` | Use supported areas and returned URLs; no invented provider setup route |

The retained Aevatar `service.connect` action-card contract is not the preferred
NyxAgent tool interface. Use the current native tools listed above and existing
hosted-link completion watches. If a tool is unavailable in the calling context,
report that limitation instead of fabricating a successful call.

## Specialist provisioning contract

Default to one specialist per selected use case: FAQ, queue and reservations.
Queue remains private to staff. The owner can choose a compatible existing agent,
but reuse must not silently broaden its authority or change its audience.

A private empty specialist can be created with existing tools. An operational
installation also needs a reviewed backend step that atomically binds the selected
service grants, explicit operation/input restrictions, customer permissions and
installation revision. Do not grant whole-service access temporarily or enable a
channel before this step. Missing scopes mean broad access in the existing model.
The current separate management tools do not establish an atomic provisioning
contract; adding that callable bounded step is required implementation work.

Store setup reference, selected skill pins, specialist IDs, connection/resource
references, approval revisions and readiness receipts in existing durable setup
machinery. The skill tells NyxBot to use this state; a transcript or remembered
checklist is not the authoritative activation record. Until a corresponding
setup API/tool exists, report that durable orchestration as blocked. On uncertain
creation, inspect state before retry; do not create duplicate agents/bots.

Current installation storage has one agent ID. Serving FAQ and reservation
specialists behind one bot/phone requires an explicit installation routing
extension that binds each permitted specialist, isolates customer context and
preserves all public execution fences. Ordinary channel relinking would replace
the target, not compose these use cases. Keep this gap visible; do not assume
multi-specialist routing exists or route customers to the owner's full NyxBot.

## Completion and implementation gates

Record separate states: needs details, needs connection, needs resources, needs
review, specialist prepared, needs test, ready to activate, active, or blocked.
These are proposed UI projections, not replacements for existing SetupState.
Skill text can recommend a step; only actual tool/provider/backend results can
advance authoritative setup and readiness. Existing owner approvals still apply.

Required additions beyond authoring these skills:

1. Authenticated public-guide-to-NyxBot handoff and durable skill-led setup tools.
2. Reviewed specialist provisioning with exact grants/scopes and installation
   binding, safe retry, and skill pin/version tracking.
3. Supported inbound telephone, queue notification and Notion intake adapters.
4. Public operational runtime and shared-channel routing for approved specialists.
5. Callable readiness/activation flows with real provider and customer evidence.

Validate the skills with dry-run owner scenarios: new owner, reusable connection,
missing scope, declined consent, existing specialist, uncertain creation, missing
adapter and incomplete channel test. No live sends/calls/writes are implied by
reading or attaching a skill. Before rollout, exercise the actual flows with
explicitly authorized test resources. Do not claim the deployment is workable
solely because these Markdown files validate.

See [existing agent skills](AGENT_SKILLS.md),
[NyxBot tooling and completion watches](chat/09-nyxbot-orchestrator.md),
[operation scopes](AGENT_OPERATION_SCOPES.md), [public BotFather](PLATFORM_BOTFATHER.md),
and the [implementation proposal](plans/2026-10-15-use-case-bundles.md).

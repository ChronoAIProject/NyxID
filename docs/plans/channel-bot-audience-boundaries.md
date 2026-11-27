# Channel bot skills and audience boundaries

Claude Fable 5.2 completed the pre-implementation review against source at
`575c5f596`. The findings in the original implementation and the agreed
estimates below refer to that revision. The six streams produced an uncommitted
runtime prototype based on `575c5f596`. The initial compatibility assessment used `dev` at
`c03a6637d`. PR #3004 ports the runtime to `c267f3dcf` and incorporates
subsequent `dev` changes through `4d3cb8902`. The per-bot setting, explicit
pilot selection and global switch control rollout. Existing installations
stay on their ordinary path until an owner opts in and the platform selects
the bot. Fable cleared the earlier static compatibility findings on the port.
Live provider acceptance remains outstanding.
[Compatibility assessment](#compatibility-with-dev-at-c03a6637d) · [Runtime PR](https://github.com/ChronoAIProject/NyxID/pull/3004)

This plan records the requested behavior for all channel bots. It does not change
runtime permissions or activate a rollout. Its requirements below describe the enabled
add-on. The [guest access specification](../CHANNEL_BOT_GUEST_ACCESS_SPEC.md) defines
the disabled path and the contract between the two implementations.

## Required behavior

1. An attached skill belongs to the bot. Every admitted conversation may use its
   pinned instructions and supporting files, including on-demand reads and
   owner-configured preloads. Skills never grant additional execution authority.
2. A guest may query information explicitly granted to that guest, a verified
   audience they belong to, or all guests. They may access their own information
   when the skill's applied policy or bot configuration permits it. Ownership
   alone does not publish a record or enable an otherwise unavailable operation.
   A service connection or a permitted GET operation alone does not publish its
   data to guests.
3. Offer a simple "Manage their own data" preset for a selected data source:
   create, read, update and delete only records belonging to the current guest.
   Each action remains individually configurable. Creating a record must
   establish trusted ownership; no CRUD operation may accept a model-selected
   owner identity. Basic guest CRUD must not require granting every guest broad
   `all` access to the underlying service.
4. Data scope and permitted actions are separate owner-controlled settings.
   Owners may narrow or deliberately broaden data sharing for a bot/source and
   select read/create/update/delete independently. Existing service grants,
   operation restrictions, approvals, billing and revocation still apply.
5. Private data requires a private authorized answer destination. A public group
   or thread does not become private because the requesting guest owns the data.
   Group answers use group-shared information by default. Private results must
   use an independently verified private conversation.
6. Guest experience is an explicit part of channel bot setup, including setup
   through a bot-assisted conversation. Setup considers skills, audience,
   information grants, own-data CRUD, identity and private/group replies before
   describing the bot as ready for guests. Choosing owner-only operation is an
   explicit completed choice; connecting a transport alone is not guest setup.

These are common product rules for every channel. Telegram, Lark/Feishu and
Discord are required acceptance targets, with the same semantics on every other
supported transport. Platform differences affect identity verification and
delivery, not the guest access model.

## Guest experience during channel setup

Use one guest-experience step within the channel setup process and the same
editor for an existing bot. Avoid a separate permissions project for a basic
customer bot. The owner should be able to configure:

| Setup choice | Owner-facing behavior |
| --- | --- |
| Who can talk to this bot? | Owner only, selected people/audiences, or guests admitted by the channel policy |
| Which skills does the bot use? | Attach skills once; their instructions and configured preloads work for owner and guest conversations |
| What information may guests access? | Select sources/resources and grant them to everyone, selected guests, or verified audiences |
| May guests manage their own data? | Choose a source and enable "Manage their own data"; optionally adjust create/read/update/delete separately |
| How does the bot recognize a guest? | Use the authenticated channel identity for bot-owned records; explain account linking when accessing an existing external account |
| Where can private answers appear? | Private authorized conversations for personal information; group-shared information for group replies by default |

Use friendly source and collection names and schema-declared ownership fields.
Do not require owners to author raw HTTP rules for the basic CRUD preset.
Only offer a working preset where the integration can enforce it. Otherwise
show the specific missing ownership capability during setup.

Before guest setup is complete, show an effective summary such as: "Guests can
read the FAQ and manage only their own customer profiles. Other customers'
profiles and internal notes are unavailable." Include a preview/test using two
distinct guest identities: A succeeds on A's records and fails on B's. Preserve
unfinished setup choices when transport authorization or account linking resumes.
Owners can save a draft or choose owner-only; unresolved guest data settings do
not silently enable access.

## Skills and configuration as policy inputs

Support both owner configuration and owner-authorized skill declarations for
guest workflows. An attached skill may describe that the bot manages the current
guest's profile or uses a particular guest-visible information source. Provide
an optional versioned, structured declaration that setup can validate and apply
to the same persisted policy used by the settings editor. Show the effective
scope and actions in the guest setup summary, regardless of their source.

Skill prose guides behavior; it cannot supply unverified account IDs, bypass
service grants or replace server authorization. Apply structured declarations
under owner/maintainer authority and existing configuration ceilings. A runtime
skill read or imported instruction cannot silently widen the active policy.
Changing or re-pinning a declaration must use the same policy revision and
authorization rules as an equivalent configuration change. An absent, malformed
or unapplied declaration grants no data access. This policy declaration format
is proposed new functionality, not a capability of today's skill packages.

## Security review of the proposed defaults

The design supports a common guest policy across channels. Four details affect
what an owner can safely promise during setup.

| Boundary | Implementation and acceptance criterion |
| --- | --- |
| Skill contents and authority to share | A guest answer can quote any skill file the model reads, including a pinned dependency. Describe that audience at attachment and when migrating existing pins. For a private package, verify the publisher or organization permits use by this bot for that audience. Reading a package for personal use establishes only personal access. Keep private customer data in an authorized data source. Skill text can suggest actions. Only an authorized configuration change can widen grants, including when a skill version changes. |
| Fields and effects within an owned record | Declare the fields guests may read and write for each operation. A customer can update their address while staff notes, roles, balances and ownership fields stay protected. Apply the ownership or sharing condition when the provider commits the update or delete. Use an atomic conditional write, provider-enforced caller permissions, or an equivalent ownership guarantee. If ownership can change between a lookup and a write, a successful lookup alone is insufficient. Exercise a transfer of ownership between lookup and write in connector acceptance tests. |
| Private context and destinations | Scope private model sessions, retained tool results and continuations to the verified guest and authorized audience. Two guests can share the same guest classification. Include cached results and secondary destinations such as posting or export tools in the audience check. Test that A's private lookup cannot surface in B's turn or in a shared destination through a later tool call. |
| Revocation, retries and older credentials | Exercise a queued mutation and a pending result delivery after the owner revokes access or the account link changes. Both must honor current authority. Test ordinary ingress and retry ingress at relay cutover. A previously issued owner-scoped relay credential must never supply broader authority for a guest action. |

The practical test for each connector is whether it can enforce these limits on
the actual account, record, fields, operation and destination. Setup should offer
the subset the connector can enforce. Selecting a shared FAQ or a person's own
profile should never publish a whole service connection.

## Findings at the original implementation baseline

| Area | Current behavior | Required change |
| --- | --- | --- |
| Skill instructions | `assistant_nyxagent::base_prompt` omits attached skills for guests | Include the bot's attached skills for every admitted audience |
| Skill preloads | `agent_skill_preload::enabled` returns false for guests | Prepare preloads through a bot-authorized bound-package reader |
| Skill execution | `handlers/agent_skills::dispatch` rejects all guest calls | Permit exact attached-package reads, keeping skill management owner/maintainer-only |
| Personal NyxBot | Guest discovery/service gates refuse service execution | Expose skill reads separately; permit data tools only under explicit guest policy |
| Specialist services | `GuestAccess::Use` permits creation and blocks changes to existing records | Separate permitted operations from shared/own-record data scope |
| Guest scopes | The guest picker selects operations, not guest identity-bound record rules | Add validated resource/ownership contracts and enforce them across execution paths |
| Generic channel relay | `channel_inbound_service` can mint owner-subject tokens scoped to the route key | Bind execution to the authenticated actor, audience and current bot policy |
| Relay authentication | `RelayAgentScope` carries service/node restrictions, without the proposed actor/resource policy | Require the same audience-aware authorization as hosted bot execution |

Relevant sources: `backend/src/services/assistant_nyxagent.rs`,
`backend/src/services/agent_skill_preload.rs`,
`backend/src/handlers/agent_skills.rs`,
`backend/src/services/agent_operation_scope_service.rs`,
`backend/src/services/channel_inbound_service.rs`, `backend/src/crypto/jwt.rs`,
`backend/src/mw/auth.rs`, and `docs/AGENT_SKILLS.md`.
The existing resource-limit proposal in
`docs/chat/09-nyxbot-orchestrator.md` explicitly does not implement guest record
ownership or guest-only resource-value rules.

## Coverage

Use one common policy contract across Discord, Telegram (including managed
bots), Slack, Lark/Feishu, WhatsApp, Messenger, Instagram, X and email where supported. Cover direct
relay, native/CMA gateway ingestion, private chats, groups/topics and followed
threads, personal NyxBot, personal specialists and organization specialists.
Apply the same contract to the existing phone/SMS/speech specialist adapters so
they cannot provide an alternate route around it. Device/custom gateway events
without an authenticated human identity may use explicitly shared information;
they cannot acquire a person's own-record access by asserting an ID.

NyxID can enforce data/tool access mediated by NyxID. External runtimes must
adopt the actor/policy contract and isolate their own credentials, memory and
data access. An external agent using independent provider credentials cannot be
certified by a change to NyxID tokens alone. Report that coverage explicitly.

## Delivery estimate and milestones

**Use 71–115 NyxID engineer-days as the working estimate for the full request.**
Codex and Claude Opus 5.5 agreed on **59–95** for the seven work packages below.
Fable estimates **12–20 additional days** for exact flag-off behavior, including
mode selection, generation changes, fleet readiness, baseline comparisons and
rollback during active work. The revised total reflects Fable's judgment.
The **weeks 4–6 hosted pilot** and **weeks 8–12 production** windows, with
midpoints of weeks 5 and 10, belong to the seven-package estimate. Fable
questioned the pilot window. Re-estimate the dates at the week-1 integration
review using the actual source, rollback test matrix and team availability.
The original estimate was 46–73 engineer-days, with a pilot in weeks 3–5 and
production in weeks 6–10. The largest revisions account for enforcing guest
identity through deferred work, setting up and previewing permissions, and
changing the relay credential contract across transports.

The earlier calendar assumed two backend engineers and one full-stack engineer, about
80% availability for this project, and access to test installations. The release
covers a common policy across NyxID-mediated channels, live setup-to-reply
tests on Telegram, Lark/Feishu and Discord, and guest CRUD for one real data
source. It includes web and bot-assisted setup. Additional connectors, native
mobile screens and independent runtimes' use of their own credentials need
separate estimates.

| Work | Engineer-days | Deliverable |
| --- | ---: | --- |
| Validate the first integration and channel identity | 4–7 | Trace the reported workflow, establish how senders map to records, inventory serving paths and check Ornn and gateway capabilities. |
| Bot skill access | 6–10 | Support reads and preloads for owner and guest turns, including private organization skills. Verify removal and revocation. |
| Policy and enforcement | 14–22 | Bind the actor and answer audience to grants, ownership and allowed actions. Enforce them through discovery, execution, queued work, continuations and private delivery. |
| First data integration | 10–15 | Implement create, read, update, delete and filtered search/list for one service. Assign ownership on create and check it on every subsequent operation. |
| Guest setup and settings | 9–15 | Save and resume guest configuration, apply skill declarations under the owner's authority, preview guest access and expose the same policy through web and bot-assisted setup. |
| Channel transports and external relay | 10–16 | Bind live and retried NyxID relay calls to guest policy, retire older bearer access and exercise the three named platforms. Check the common default on other supported paths. |
| Migration, acceptance testing and rollout | 6–10 | Inventory existing pins and grants, notify owners about changed skill visibility and data sharing, test isolation, pilot on existing and new bots, and verify revocation and rollback. |
| Additional work for exact flag-off behavior (Fable) | 12–20 | Keep the existing path callable. Prove that a fresh request works on a former pilot bot while the previous add-on turn settles, including across workers and relay retries. |

**Week 1.** Use the reported Discord workflow to identify where the record
lives, what proves ownership and which identity is available on each channel.
Walk one create, list, update and delete request with two test customers. Check
whether Ornn can authorize a private skill for the bot and the CMA gateway can
vouch for the sender and reply audience. Re-estimate the integration if the
source lacks a stable ownership field, cannot filter lists/searches at the
server or needs a new identity link. Allow **2–5 additional engineer-days each**
if Ornn or CMA needs changes in its own repository, plus time for the respective
team's release. Record a test matrix for all registered transports.

**After the week-1 review.** Integrate the common turn and policy interfaces
with the first skill and data workflows. The first independently demonstrable
milestone is a bot answering both an owner and a guest using an attached file
it reads on demand.

**Hosted pilot acceptance.** Use one real data source. A bot owner completes
setup, shares one resource with A, and enables private CRUD. Both guests can
use the bot's skill. A can manage A's records, B cannot discover them, and a
group request for a private record continues in a verified private conversation.
If the platform has no verified private destination, ask the guest to message
the bot privately before fetching any personal data. This milestone includes
read, list, create, update and delete through the real service.

**Production acceptance.** Exercise the same journey on Telegram, Lark/Feishu and Discord,
including direct and gateway paths where available. Verify the common default on
other adapters and NyxID-mediated external relay calls. Review configurations
with bot owners during the pilot, including older private skill attachments
whose contents could now inform guest answers. Roll out with separate evidence
for transport delivery, skill availability, data access and reply privacy.

**Connector expansion.** Scope each further data source by its ownership and
search APIs. Budget roughly **1–2 engineer-weeks** for a straightforward
connector after the common policy ships. Shared workspaces with folders,
inherited access, search, exports or mutable ownership can require several weeks
each. The earlier [resource limits proposal](../chat/09-nyxbot-orchestrator.md#resource-limits-for-guests--proposal-not-implemented-1996)
gives comparable estimates for resource selectors and provider-specific checks.
Independent external agents need their own adoption estimate wherever they
access data with credentials they control. A universal CRUD estimate requires
an inventory of those integrations.

## Parallel implementation assignments

Plan for **five sustained streams, with six as the practical peak** for one
data source and three live channel targets. With six implementers, give each
row below one owner. The channel engineer can start on day one if test
installations are available. Their greatest contribution comes when the common
implementation is ready to exercise through real channels. These assignments
divide the 71–115 engineer-days already estimated.

| Owner and scope | Independent first task | Handoff and acceptance |
| --- | --- | --- |
| 1. Runtime and rollback (backend) | Capture ordinary hosted behavior. Specify the trusted turn record, global selector and activation generation. | Give policy, skills and channels the turn interface. Own admission, session isolation, stale-worker cancellation, event deduplication and fleet readiness. After switch-off, a new message in the same thread completes via legacy code while old work settles, with one provider effect and one billing settlement for the original action. |
| 2. Guest authorization (backend) | Write decision cases for two guests, a named grant, own-record access and group versus private answers. Specify evaluator inputs and outputs with the runtime owner. | Give the connector and setup owners one policy schema and one evaluator for preview and execution. Own shared MCP/proxy dispatch and rechecks before deferred actions or delivery. A revoked grant stops pending access and private content never enters a group turn. |
| 3. Attached skills (backend) | Trace an on-demand read and a preload for a personal and an organization bot. Establish what Ornn can authorize for the exact pin. | Supply the bot-authorized reader and discovery changes, using the runtime turn and the authorization owner's shared dispatch hooks. Owner and guest can read the authorized pin under the agreed approval setting. Removing it stops both read paths. |
| 4. First data source (integration backend) | Identify the actual source behind the Discord report. Verify the owner field, server-side search filtering and the write-time ownership guarantee with two test accounts. | Give setup the supported actions and fields. Implement the provider operations behind the common evaluator. A can create, list, read, update and delete A's permitted data. B cannot discover it, and an ownership change prevents a stale mutation. |
| 5. Guest setup (full-stack) | Map existing web and bot-assisted setup. Draft the owner-only, shared-resource and own-data configurations against the policy examples. | Own draft/resume, skill declarations, the editor, preview endpoint, inventory and owner notice. Use the common evaluator and connector capabilities. After resuming setup, the two-guest preview agrees with execution on the real source. |
| 6. Channel integration and relay (backend with integration test experience) | Confirm test installs. Capture ordinary ingress, retry and reply behavior on Telegram, Lark/Feishu and Discord. Inventory sender and destination evidence on the other adapters. | Supply verified identities and reply destinations to runtime and policy. Own adapter wiring, both relay credential paths and the cross-channel test suite. Exercise private and shared conversations, retries, and flag-off parity on the three live targets, plus the common default on other paths. |

**With five implementers**, assign the first five rows. During the skill work,
the runtime owner includes direct and retry ingress in baseline capture, and
the setup owner obtains the channel test installations. Move the skills owner
to channel integration and relay after the pinned-read milestone. If private
Ornn authorization is waiting on the other team, that engineer can start the
relay inventory or transport fixtures during the wait. With a sixth available
from kickoff, assign that work directly and run it alongside skills.

**Integration order.** In week 1, runtime and authorization publish a shared
example of one admitted turn and its access decision. The connector owner
confirms which operations the real source can enforce. Setup and skills can
build against those examples as the interfaces settle. Use the enrollment
and pinned-read rules in the runtime spec. The runtime owner integrates changes to shared
admission and worker code. The authorization owner integrates changes to shared
tool dispatch. Each adapter supplies the verified facts those interfaces need.

The critical path then runs from trusted admission through live authorization
and the real connector to a hosted two-guest pilot. Production also needs the
relay credential tests, three-platform runs and rollback with active work.
A seventh NyxID implementer would mostly share those integration points.
Separate Ornn or CMA repository changes can run concurrently with their own
teams. Reforecast pilot and production dates at the week-1 review using the
actual source, platform access and owners available.

## Implementation slices

### Fable's pre-implementation review

Fable conditionally supports starting slices 1 and 2 behind the disabled flag
once the kickoff decisions below are recorded. The runtime spec now defines
global-only selection, trusted turn identity, separate conversation rows and
rollback during active work. Three points affect how the team proceeds.

| Decision | Disposition |
| --- | --- |
| An attached skill is read when the owner requires approval for all requests | For implementation, use explicit, revocable standing authorization for this bot, audience and exact pinned attachment where the maintainer has approval authority. Otherwise require the normal applicable approval. Cover on-demand reads, preloads, revocation and private organization packages. |
| An existing bot opts in with no applied guest data policy | The saved bot setting defaults to off. Once its authorized owner or administrator opts in and the rollout permits it, the bot starts with zero guest data grants. Authorized skills become available and owners can share resources or enable own-data operations in setup. |
| Personal NyxBot data and external relay | Fable suggested a skills-only first release for personal NyxBot and questioned relay coverage. The requested scope still includes policy-bound personal data tools and NyxID-mediated relay. Prove both the durable agent key and relay tokens are constrained before enrolling a relay binding. A platform pilot can establish hosted behavior first. |

The implementation already provides a global-only flag check and a transaction
for flag mutations. Hosted execution currently carries a guest boolean.
The relay's reply handler separately authenticates the assigned agent API key.
[Flag administration](../../backend/src/services/feature_flag_admin_service.rs) ·
[Turn authority](../../backend/src/services/assistant_acknowledgement_service.rs) ·
[Channel reply authorization](../../backend/src/handlers/channel_relay.rs)

At the week-1 review, set a measurable baseline for same-thread legacy admission
after switch-off, bound stale-worker cancellation, and document recovery for each
transport when an authoritative flag read fails. Check org-specialist rollout
and payer selection, the private Ornn read, the real record source and private
delivery on each acceptance platform. Use these results to update the calendar.

### 1. Shared actor and audience contract

Derive an internal actor from authenticated provider events and the exact
installation/tenant and bot binding. Keep platform account IDs separate from
editable names and from downstream application account IDs. Cross-bot or
cross-platform identity linking requires explicit verified mappings; matching
names, email text or supplied IDs never creates a link.

Carry the actor, identity assurance, bot/binding, conversation, source event,
answer audience and policy version through admission, model preparation, tool
calls, async continuations and delivery. Verified owners/members retain their
existing identity and permissions; public delivery still requires an appropriate
audience. Bots and unverified senders never inherit human owner authority.

Reuse existing verification and organization-link records where they prove the
required mapping. Missing identity permits only explicitly shared data, not
access to claimed existing personal accounts.

### 2. Bot-owned skill reads

Implement guest instructions, discovery, dispatch and preloads inside the
add-on using a narrow, live, bot-authorized bound-skill read. This must also work
when personal NyxBot guest discovery otherwise removes tools. Keep search,
attachment, editing, publication and other management operations under their
existing owner/maintainer checks.

Read only the active bot's attached root/dependency GUID, exact version and
verified archive hash. Preserve package/file bounds, safe paths, revocation,
explicit operation restrictions, live turn/key validation and context reset
when pins or preloads change. Skill contents remain untrusted guidance.

Design the Ornn authorization for owner-delegated bot use explicitly. Existing
organization skill membership capabilities reject guests; deleting that check
would manufacture member authority. Prove a purpose-limited bot read using
legitimate attachment authority, including organization-private skills and
revocation. If Ornn requires a corresponding API change, include it in this
slice before claiming organization coverage. No general Ornn or account token
may be exposed to the guest.

### 3. Shared/own-record data authorization

Add a versioned policy for each bot/source with separate audience/data scope and
operation permissions. Support explicit shared resource IDs or bounded
collections and provider-declared ownership rules resolved against the current
actor. Distinguish "no guest access", "shared information", "own records" and an
intentional owner-selected broader source scope.

Support grants to an individual authenticated guest as well as to all guests or
verified audiences. Check current grant membership per request; a resource
explicitly granted to A is not thereby granted to B. Own-record access is also
subject to the effective skill/config policy, not merely a matching identity.

Compile the own-data CRUD preset into trusted ownership assignment on create,
filtered reads/lists, ownership checks on updates/deletes and protection against
reassigning ownership. For bot-owned records, a provider-authenticated guest can
have a stable internal identity without creating a separate NyxID account.
Access to pre-existing external personal accounts still requires a verified
mapping. Anonymous or weakly identified callers cannot claim those accounts.

Extend the existing input-rule engine for fixed resource constraints. Dynamic
ownership needs a trusted mapping or a provider query/check; a literal path or
body rule alone cannot prove ownership. Select one real affected integration
for the first end-to-end ownership adapter, then expand provider support.

Filter searches, lists, pagination, aggregates and related references before
results enter model context. Constrain direct reads, downloads, exports,
attachments, batch operations, async results and mutations by the same policy.
Where an integration cannot establish an allowed result set, refuse the
operation under the default policy. Do not fall back to unrestricted search or
fetch everything and ask the model to hide other people's data.

Enforce the policy for MCP and raw proxy calls, concrete pool members, relevant
LLM/tool routes and final provider dispatch. Live policy/binding revocation must
reach existing threads and queued work. Give external relay calls a short-lived,
actor/audience/policy-bound capability and remove broad owner-token fallback for
guest work. Metadata headers alone are not execution authority.

### 4. Guest setup, owner settings and existing bots

Present data scope and action permissions separately in the existing bot/agent
settings. Identify shared sources/resources and explain whether own-record
support is verified for that integration. Reuse bot defaults with explicit
source overrides and revision checks. Ordinary chat requests cannot widen
these settings; authorized owner/maintainer changes use existing management
authorization and audit paths.

Implement the guest-experience step in both new-channel onboarding and existing
bot settings, including personal/organization bindings and bot-assisted setup.
Reuse one effective policy summary across UI, setup tools and API/CLI surfaces.
Update `docs/assistant/CHANNEL_SETUP_GUIDE.md` and its tool guidance when the
runtime capability exists; do not advertise the proposed options in the live
guide before implementation. Extend guest readiness beyond model availability
to report applied information grants, identity/ownership support and configured
CRUD behavior. Treat the preview as a configuration check and record actual
two-guest scenario test results separately from model or transport readiness.

Retain current grants and operation selections as ceilings. Do not interpret
legacy `use`, `all`, `allow_all_services` or a connected service as proof that
every record is guest-shared. Inventory affected configurations and show owners
which guest queries require a resource or ownership rule. Notify owners that
attached skill contents can inform guest answers and that existing service
grants alone do not share records. Keep existing bots on ordinary settings
until an authorized owner or administrator opts in. Set the preference on
when a new bot is created. Apply the guest data policy only when the saved
preference, platform flag and rollout scope permit it.

### 5. Verification and rollout

Start with failing behavioral tests, then implement each slice. Test the common
evaluator and representative real ingress-to-tool-to-delivery journeys. Required
cases include:

- Owners and guests both read the same attached skill and configured preload;
  removed, disabled, revoked and tampered packages are refused. Guests cannot
  manage skills or use the read exception for arbitrary Ornn calls.
- Guest A can create, read, update and delete A's records under the own-data
  preset, without permissions over B's records or ownership reassignment. A
  cannot read, discover, count, export or mutate B's records by changing an ID,
  search query, batch, alternate route or async result reference.
- A resource granted only to A is unavailable to B; broader audience grants and
  revocations are applied consistently. Matching ownership does not bypass a
  disabled operation or missing skill/config permission.
- Unverified identities can read explicit shared sources and cannot claim a
  personal account. Spoofed display names, quoted messages and bot authors never
  establish owner/member identity.
- Private own-record results do not enter group prompts, transcripts or replies.
  Owner-to-guest transitions and other guest sessions do not inherit private
  context or cached results.
- The same decisions apply to direct/gateway/native routes, personal/org bots,
  external relay tokens, relevant adapters and final proxy/MCP dispatch.
- Telegram, Lark/Feishu and Discord each pass the same skill, named-guest grant,
  own-data CRUD and private/group audience scenarios through their real channel
  entry paths. Channel setup and its resume expose and preserve all guest
  choices; no transport silently inherits broader guest defaults.
- Structured skill declarations and direct configuration produce the same
  effective policy and preview. Runtime prose, unsupported declarations and
  unauthorized skill/config changes cannot widen it.
- Owner overrides, policy revocation, in-flight changes, legacy capabilities and
  mixed-version servers cannot bypass the boundary. Existing billing principals,
  org guest budgets, approvals and reply routing remain correct.

### Runtime feature flag and rollback

Implement this as an optional module behind the default-off global runtime flag
`nyxbot:guest-audience-v1`. Off routes every fresh request to the existing
implementation with current ordinary settings, including previously enrolled
bots. With the flag on, the rollout can activate only bots whose saved setting
is on. Exercise a pilot with one new bot and an older bot owned by the same
account, then widen to all eligible bots. Keep both paths callable,
and store add-on policies, private sessions and work separately. Turning off
requires one flag change and permits fresh legacy work while add-on work settles.

Capture baseline behavior before the first runtime change. Compare the off path
before enrollment and after on → off against that baseline on Telegram,
Lark/Feishu and Discord. Include outstanding retries and committed provider
effects across the switch. The [runtime contract and acceptance criteria](../CHANNEL_BOT_GUEST_ACCESS_SPEC.md)
are authoritative for code boundaries, state isolation and rollback.

## Implementation and acceptance

On the prototype based on `575c5f596`, the hosted runtime, guest policy, pinned
skill reader, own-record connector, owner setup and external relay contract
are implemented behind the default-off
flag. NyxID's bot-owned Personal records collection is the first CRUD source.
A related Discord workflow has a separate bounded Sheets read and whole-tab
append integration. Its connection and workbook have not been identified as
the installation from the original report. [Related Discord workflow](https://github.com/ChronoAIProject/NyxID/issues/2704)

The original server binary passed 66 selected legacy cases. The branch passed
the same 66 cases and the additional hosted organization-guest case in a
67-test comparison. One legacy cold-start billing check initially observed
one model call where it expected two. It passed in isolation and in the full
67-test rerun. On the branch, 8,298 frontend unit tests and 22 Chromium cases
passed, including guest setup, draft/resume, skill audience and approval settings.
The browser tests use HTTP fixtures. Backend integration tests run against a
local MongoDB replica set. All 51 focused feature tests passed. The run
includes private and organization-owned skill reads, dependency files,
preloads, revocation, the org payer, policy authoring, external relay retries,
and a fresh same-thread legacy answer with the old hosted worker held open.
Another 368 tests passed across adapters, routing, admission, calls and
feature administration. These suites overlap. The server and its tests also
passed Clippy with warnings treated as errors.

The local external-relay test passes Telegram, Lark, Feishu and Discord
Interactions payloads through their parsers, channel ingress and the guest
tools HTTP endpoint. On each transport A creates and reads a record, B is
refused access to A's record, and a shared chat cannot discover or call the
personal-record tools. Turning the flag off restores ordinary callbacks for
fresh events and rejects the earlier guest credential. The hosted MCP tests
also exercise field projections, stale credentials and live approvals.

Production acceptance still requires an authorized Discord, Telegram and
Lark/Feishu installation, the customer's actual source, and a publisher-side
check for an organization-private Ornn package. Confirm provider delivery and
credential behavior during rollback on the pilot fleet. Use the preactivation
inventory to identify affected owners and send the notice before enrollment.
The prototype's operator guide is `docs/assistant/CHANNEL_GUEST_SETUP.md`.

### Fable's post-implementation compatibility review, November 26

**Verdict: approval withheld, including for a merge with the flag off.**
Claude Fable 5.2 reviewed the working tree against `575c5f596` and inspected
the saved test logs. This was a code review. Fable did not run additional tests.
Ordinary owner/member turns, pre-upgrade rows, legacy callback payloads and
reply-token claims passed static inspection. The following findings remain open.

| When | Finding and user impact |
| --- | --- |
| Flag off, ordinary external reply | The reply handler consumes the single-use token before a second database-backed delivery check. If that check fails transiently, the provider receives no reply and the agent's retry is refused. |
| Turning the flag on | An external callback delivered under the legacy rules can lose its outstanding reply. A pending legacy Aurinko callback can return 503 repeatedly until the provider stops retrying. The review found no duplicate send on either transition. |
| First activation | An absent pilot selection selects every bot. A single global toggle can change every external relay and refuse guest calls on phone, SMS and speech bindings. Fable calls for an explicit selection on first activation, including an explicit choice for a fleet-wide rollout. |
| Flag off, provider webhook | Immediate acknowledgements and the managed platform dispatcher now require a successful selector read within 650 ms. Database delay can produce a provider-visible 503. Failing closed when authority is unknown is specified. Applying this timeout before acknowledgement changes ordinary availability. |
| Flag off, batches and lifecycle | A selector error can abandon the remaining messages in a batch without the usual diagnostic. Ownership transfer and deletion also depend on feature storage for ordinary bots. |

For enrolled external relays, the new contract applies even to the owner's
channel messages. Callbacks lose the ordinary owner bearer and media payload,
and the assigned key loses general proxy/model access after it is bound.
Operators need this information when selecting existing external bots.

Fable requested full backend coverage, tests for off-to-on retries and replies,
selector failure before and after token consumption, representative legacy
API keys and child credentials, and mixed-version rollback. The existing
three backend test groups cover 483 distinct cases. The 67-test parity
rerun passed, but the cause of the earlier cold-start billing discrepancy is
still open. There are no measured acknowledgement-latency comparisons or
live three-platform acceptance results. Fable's sign-off requires resolving
the high-priority findings and the flag-off acknowledgement behavior.

### Compatibility with dev at c03a6637d

The November 26 comparison used the uncommitted prototype at `575c5f596`
and the fetched `dev` revision `c03a6637dcfa520609bd9f9db02b7f9200641ca5`.
The 451 intervening commits touch 76 of the 81 tracked files changed by the
prototype. A three-way merge reports 52 conflicting paths. These counts measure
integration work. The tests reported above ran against the older baseline.

**User-visible effects of enrollment**

| Who is affected | What changes | Required evidence before enrollment |
| --- | --- | --- |
| Discord org guests with working Sheets grants | Current Workspace and standalone Sheets fixtures cover guest append and a bounded read under applicable service and connection-owner approvals. The add-on starts with zero data grants. The owner must explicitly share the range and authorize the possible whole-tab insertion to keep the workflow. | Replay the two catalog configurations separately with both settings of the flag. Confirm `Event_Config!A1:B2`, `Questions!A:H`, RAW/INSERT_ROWS, owner writes and denial under an unapproved connection. The fixture identifies a related workflow. The reporting customer's actual configuration still needs verification. [Current integration tests](https://github.com/ChronoAIProject/NyxID/blob/c03a6637d/backend/src/services/org_channel_service/grants_cutover_tests.rs#L922) |
| Personal hosted bots with guest media enabled | The prototype discards enrolled guests' media selectors. A guest who can submit an image or document on a supported personal relay today gets a text-only turn. | Include the media opt-in in the pilot inventory. Test the same private and group workflow before, during and after enrollment. [Guest media contract](https://github.com/ChronoAIProject/NyxID/blob/c03a6637d/docs/chat/10-uploads.md#personal-channel-guest-media) |
| Custom external agents and their owners | The callback becomes text-only and loses the ordinary owner bearer even for the owner's own message. After the first binding the assigned key loses general model/proxy access. An external agent also needs a verified specialist association for hosted skills or Sheets. | Identify integrations that depend on attachments, thread context, model access or shared keys. Verify ownership, ordinary sibling routes, the original callback reply and fresh ingress after rollback. |
| Bots already in a running pilot | The prototype's `set_pilot` rejects membership edits while enabled. Adding one bot requires a global off/on cycle, which cancels pending add-on work for bots already in the pilot and briefly restores their ordinary grants. | Show the affected bots before the change. Exercise an expansion from A to A+B with pending work on A and verify the documented rollback behavior. |

**Upstream contracts to preserve during the port**

| Area | Concrete integration risk and check |
| --- | --- |
| Organization specialists | `dev` separates use, maintenance and governance and fences revocation. Wiring an earlier owner/admin check into setup or execution can grant or withhold the wrong operation. Exercise an admin excluded from specialist use, a selected non-admin maintainer, a use-only member and a verified guest. Check each operation's bot-management and specialist rights together. [Specialist authorization](https://github.com/ChronoAIProject/NyxID/blob/c03a6637d/backend/src/services/org_agent_service.rs#L53) |
| Messenger and Instagram | Both customer inboxes ship on this baseline. An ordinary private DM cannot enroll an owner or turn on customer exposure. Map the verified sender to the exact inbox and grant, honor the 24-hour limit and live control at send, and include new guest data in scoped deletion. Confirm that deleting one inbox preserves the other. [Meta channel contract](https://github.com/ChronoAIProject/NyxID/blob/c03a6637d/docs/META_CHANNELS.md) |
| Retained source and reply state | Current source readback on the conversation messages endpoint can return sender ID, response ID and delivery status. Preserve ordinary management access. Keep private add-on references out of other guests' results, and check queued or resumed work after revocation, rollback and route reassignment. [Source result lookup](https://github.com/ChronoAIProject/NyxID/blob/c03a6637d/backend/src/services/channel_source_result.rs) |
| Encrypted queues and Discord replies | The prototype encrypts the new guest queue payload with the generic envelope. Current event storage binds ciphertext to record, owner and field and tracks migration and verification. Discord native reply handles also have record-bound storage. Port the new field into these contracts and test key rotation, transfer, restart and legacy records. [Event fields](https://github.com/ChronoAIProject/NyxID/blob/c03a6637d/backend/src/services/nyxbot_event_record.rs) · [Native reply handles](https://github.com/ChronoAIProject/NyxID/blob/c03a6637d/backend/src/services/platform_native_reply_record.rs) |
| Provider requests and callbacks | Current outbound paths bind connections to checked destinations. A port that builds a second client for a skill read, Sheets call or callback can lose that check. Exercise the current destination resolution, connection, redirect and approval checks at final send. [Callback delivery](https://github.com/ChronoAIProject/NyxID/commit/af59eb464) · [Direct proxy](https://github.com/ChronoAIProject/NyxID/commit/caa7d86eb) |

Port the selector, outstanding reply/retry and lifecycle fixes from the
spec's acceptance criteria first. Compare the baseline to the integrated
build with the flag off and outside the pilot, including injected storage
failures and ACK latency. Then test enrollment, both cutover directions and
the enrollment effects above. The release evidence still needs the full
backend comparison, live Discord, Telegram and Lark/Feishu runs, and a fresh
compatibility review of the integrated code. The original estimate has not
been revised to include the port.

### Pre-merge comparison with production main

The latest recorded production rollout deployed `898fdf245` on November 23.
Its NyxBot check recorded `skipped` during configuration. Fable compared the
affected areas of that revision, the docs commit based on `dev`, and the
uncommitted runtime prototype.
Fable withheld runtime approval on either branch, including with the flag off.
The review used source inspection and patch applicability checks. It included
no live channel tests or customer setup census. [Production rollout](https://github.com/ChronoAIProject/NyxID/actions/runs/45115192660/job/137512830713)

Production has a test for organization guests discovering the allowed standalone
Sheets operations after the binding adopts specialist grants. The more complete
Workspace and standalone Sheets execution tests cited above are on `dev`.
The customer report remains open. [Production test](https://github.com/ChronoAIProject/NyxID/blob/898fdf245/backend/src/services/org_channel_service/grants_cutover_tests.rs#L710) · [Discord report](https://github.com/ChronoAIProject/NyxID/issues/2704)

Existing service connections, grants and pins stay stored. Enrollment adds a
different access decision. The prototype offers no automatic conversion of
guest levels, operation selections or named audiences into scoped data grants.
The first supported personal CRUD collection starts empty. Many legacy service
operations have no replacement in v1. At first enrollment the prototype permits
enabled hosted pins for both audiences, subject to publisher and approval
checks, and exposes the editor only after activation. This can interrupt a
working service and make existing instructions quotable in the same first turn.
The spec requires pre-enrollment preparation and an atomic switch to the
selected revision. Implement and verify that path before enrolling existing bots.

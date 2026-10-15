# Use-case operating skills and owner customization

Status: proposed extension to the onboarding design. Existing authoring primitives
are identified below; copy-on-setup, installation version binding and the complete
interactive lifecycle are not implemented by these documents.

## Two distinct skill roles

Every initial use case needs both onboarding guidance and default operating
guidance. The onboarding skill belongs to authenticated owner NyxBot and guides
connection, provisioning and verification. The operating skill belongs to the
resulting specialist and defines its repeatable work after setup. Attaching the
onboarding skill to the specialist does not supply its operating behavior.

| Use case | Owner onboarding skill | Specialist operating guidance |
| --- | --- | --- |
| FAQ | Collect reviewed facts and connect Telegram/telephone | Answer from published facts, handle unknowns, respect customer context |
| Queue | Connect queue source/SMS sender and verify staff operations | Read authoritative queue status, identify the selected entry, invoke bounded staff-requested alerts, explain send state |
| Reservations | Connect channels and choose/map Notion destination | Collect required request details, record pending requests, distinguish receipt from confirmed booking |

The three existing `setup-restaurant-*` files are onboarding drafts. Three
separate operating packages still need to be authored, reviewed and published.
They must reference only implemented advertised operations and cannot grant
access, create missing business adapters or override enforced policies.

Extend admin use-case definitions with separate `onboarding_skill_pins` and
`default_operating_skill_pins`, target specialist roles and a customization mode.
Resolve them under the owner's live Ornn visibility. Version the definition;
existing setups retain the exact selected version.

## What NyxID already implements

Under `assistant:agent-learning` and applicable pilot/access checks, NyxBot can
use `nyxid__draft_agent_skill` for an agent it maintains. Specialists can draft
for themselves. Owner-requested drafting does not require automatic learning
enrollment. Auto and Ask every time show a full owner card; approval packages,
publishes privately, verifies the package and pins it to the target agent. Full
access publishes and pins immediately with an automatic receipt through the same
validation and ownership fences. Subsequent
eligible improvements publish a new version and update that agent's pin.

This is already a publish-and-pin operation. It does not stage a candidate for
pre-activation testing on a live agent. Chat text such as “allow” does not approve
publication in reviewed modes; Telegram can link to the exact review card or
automatic receipt. Signup alone does not select Full access.

Native `preview_agent_skill` and `get_agent_skills` return references/metadata,
not another specialist's full source. `skill_read` reads only the calling agent's
attached packages. NyxBot can delegate a bounded improvement request to the
specialist, which reads its own skill and drafts its own revision. Alternatively,
add a maintain-authorized authoring-source reader; do not attach every specialist
operating skill to NyxBot just to obtain its text. Initial exact copying likewise
needs a server-side source loader under the owner's registry identity.

The current improvement path requires an attached base with no pinned
dependencies, private visibility, matching personal owner, no sharing and the
exact latest version/hash. It preserves the supported base interface. Text-only
authoring is bounded to 7,500 Unicode characters / 30,000 UTF-8 bytes including
metadata and permits Markdown/text supporting files. It cannot directly edit a
shared platform default or publish an org-owned customization through a personal
fallback. See [existing agent skills](AGENT_SKILLS.md).

Agent-submitted drafts include the advertised `scope` declaration: complete or
reduced, with explicit omissions. Reduced scope needs separate consent in reviewed
modes; Full access records its automatic handling. Preserve the requested procedure
or disclose omissions rather than silently shortening it. Respect the bounded
Ornn format-repair budget and reconcile uncertain publication instead of retrying.

## Recommended initial ownership model

For these three use cases, prepare small dependency-free text operating packages
that the platform is authorized to let customers copy and customize. At initial
setup, materialize an owner-private operating copy for each specialist through
the existing authored publication/review machinery. Record its exact source pin
and content digest; preserve required attribution. Never send a shared platform
default to the existing in-place improvement path.

This copy-on-setup orchestration is new work. Reuse existing packaging,
validation, review and publication services; do not ask the model to reconstruct
an exact copy when a verified package transformation can produce it. Any generated
adaptation is available on the owner review surface or linked receipt. Keep live credentials and resource bindings
in authoritative setup configuration rather than copying them into skill text.

Publish the private copy under the effective mode before the specialist becomes
operational: owner card in Auto/Ask every time, automatic receipt in Full access. After successful publication/pinning and setup tests, record the
reviewed exact pin in the installation revision. Failed/uncertain publication
must reconcile via the existing publication operation before another copy is
attempted. Resume must not create a new private package or specialist.

Existing specialists already attached to a shared default need an explicit
first-customization transition. Creating a new private skill currently adds a
pin; it does not automatically replace an unrelated default GUID. A bounded
reviewed replacement must preserve unrelated pins, remove the replaced default
only after the new publication is verified and bind the expected skill revision.
Do not leave contradictory old and new procedures active together.

Copy-on-first-edit can reduce initial publications, but needs that replacement
transition and provenance equally. The initial proposal chooses copy-on-setup to
make later owner-requested improvements use the existing private-base path.

## User experience: test, give feedback, improve

Example: the owner tests the queue specialist and says, “Its queue report is too
long. Show the next three entries with party size and waiting time.”

1. NyxBot resolves the exact setup/use case, specialist and current operating pin.
   It checks whether the problem is wording/procedure, configuration, permissions,
   an adapter failure or missing data. Queue length and waiting time must come
   from the authoritative source; changing prose cannot supply unavailable data.
2. For a skill change, NyxBot delegates an owner-requested improvement to the
   specialist, which reads its own eligible private base and drafts the revised
   procedure using existing authoring tools. A future maintain-authorized source
   reader could also support direct NyxBot edits. Explain the behavior
   change and preserve unaffected instructions. For a configuration change such
   as the return window, it uses the setup configuration tool instead.
3. Follow the effective mode: review exact files on the existing card in
   Auto/Ask every time, or publish immediately with a receipt in Full access.
   NyxID pins the verified new version. No manual Markdown editing, ZIP upload or
   reattachment is required for eligible improvements of the same private skill.
4. Record the resulting pin and invalidate affected readiness evidence. Re-run
   appropriate test cases; distinguish a test pass from publication success.
5. Activate/resume the reviewed behavior and expose its version and previous
   version in setup history. Confirm successful application only from persisted
   publication and pin results.

“Automatic” means NyxBot prepares the edit and NyxID publishes and attaches it.
Auto and Ask every time require full owner review. Full access already supports
immediate publication and pinning without that review. Installation suspension,
tests and activation remain independent product gates; Full access does not
implement them or bypass source ownership/integrity checks.

Changes to SMS return windows/templates, booking rules or business hours must
update their authoritative configuration when applicable. Skill edits cannot
widen grants, create tools or override server-enforced notification templates.

## Live behavior and testing boundary

For a new, inactive specialist, publish/pin the operating skill, test it, then
activate the setup. For an already active specialist, the existing authoring
flow changes the agent's pin immediately on success; it does not wait for setup
activation. Therefore the installation layer must enforce one explicit strategy.

The minimum first release pauses affected operational execution, lets in-flight
work settle, then applies the reviewed edit, tests and resumes. Preserve unrelated
ready use cases and reconcile any committed external effects. Direct pin changes
outside the setup panel must also be detected: pin drift blocks that capability
until its installation revision is reviewed and tested. The execution path must
check this boundary, not rely on the panel's status.

Pausing an onboarding setup alone does not stop an ordinary private specialist.
The suspension must cover every affected execution entry, including owner
delegation, direct chat, channels, triggers and queued work. An explicit restricted
test path is needed while suspended; otherwise a pause would also prevent the
tests needed to resume. This enforcement is new implementation work.

Enforce suspension and draining as a server-side precondition of publishing and
applying a new pin to an active setup-bound specialist. Its owner card can be
approved independently of the setup panel, so conversational ordering is not an
execution fence. In reviewed modes, drafts can be prepared while the previous
behavior remains active; the review/apply transition must establish suspension.
In Full access, draft_agent_skill can publish immediately, so establish the gate
before invoking it, including delegated drafting. Enforce the precondition in the
shared publication/pin service for both modes; caller ordering is insufficient.
Pin-drift checks remain the backstop for other modification paths.

Suspension blocks business execution, while a bounded maintenance turn may read
the exact operating source and prepare its authored draft. It cannot send SMS,
write reservations or perform other business effects. Restricted test turns use
only explicitly approved test resources. Keep these maintenance/test authorities
separate from normal direct chat, delegation and channel execution.

A later uninterrupted-edit mode can keep an active exact skill version while a
candidate is tested with controlled test resources, then switch atomically. That
requires candidate skill selection and execution routing; it is not supplied by
today's publish-and-pin operation. Do not label current editing as staged rollout.

Rollback explicitly reselects a verified previous pin under the effective mode,
rechecks its permissions/availability and retests affected behavior. Pinning an
older version may make the existing next-improvement path refuse because the base
is no longer latest; require reconciliation or a reviewed new private lineage.

Today's generic guest turns omit skills, and existing platform customer inference
ignores attached skills. FAQ/reservation customer behavior therefore needs the
planned operational runtime to consume only the activated operating guidance
without exposing owner context or setup tools. Attaching a skill alone does not
change public Telegram/telephone behavior. The private staff queue path can use
owner specialist guidance, subject to its actual supported business tools.

## Provenance and shared default updates

Record per specialist/use case:

```text
use_case_definition_revision
source_default_pin: source, skill_id, version, sha256
owner_operating_pin: source, skill_id, version, sha256
specialist_id, customization_lineage_id
last_publication_operation_id
tested_pin, approved_installation_revision
previous_active_pin, customization_status
```

Admin updates publish a new shared default and a new use-case definition revision.
New setups select that revision. Existing private copies stay pinned until their
owners accept a proposed update. Compare the previous default, new default and
owner-customized version; present conflicts and preserve local changes. This
comparison/migration orchestration is new work. Never silently replace a user's
customized procedure or mutate an immutable pinned package.

One private lineage per specialist avoids unexpected cross-specialist edits.
Publishing a new version does not advance pins on other specialists automatically.
Background learning remains a separate opt-in feature, with its existing evidence
and owner-review constraints; customer conversations do not silently become
training or improvement evidence.

## Implementation boundaries and acceptance

1. **Admin catalog and setup sessions:** dynamic definitions, selection/handoff,
   durable progress, interactive cards and exact onboarding/operating references.
2. **Specialist skill lifecycle:** initial private copies, provenance, default-pin
   replacement, existing authored improvement/review integration, pin-drift
   execution checks, retest/resume and explicit rollback.
3. **Operational use cases:** queue/SMS and reservation/Notion adapters, inbound
   telephone, bounded public skill execution and shared-channel routing.

These can be reviewed in separate PRs. The first two deliver reusable onboarding
and customization infrastructure; the full restaurant promise also requires the
third. No new authoring engine is needed for supported personal private skills.

Acceptance includes initial copy/retry deduplication; default version visibility;
owner feedback producing an improved private version on the right specialist;
shared-default improvement refusal; stale pin conflicts; uncertain publication;
no contradictory default/custom pins; denied/expired review; unchanged service
authority; live pin drift fencing; retest/resume; rollback; default upgrade
conflicts; unrelated agent pins preserved; and customer runtime using only its
approved operating guidance. Disabled authoring or unsupported ownership must
produce actionable unavailable states rather than claims of successful editing.

See [interactive setup sessions](NYXBOT_ONBOARDING_SESSIONS.md),
[onboarding skill template](USE_CASE_TEMPLATES.md), and
[restaurant requirements](RESTAURANT_USE_CASES.md).

# Agent details implementation plan

**Owner/reviewer:** the primary Codex session. **Implementer:** one Heca Codex session using `gpt-6.1-sol`, `xhigh`, `full-access`. The primary reviewer performed its own implementation review. On the user’s subsequent explicit request, a separate fresh Heca Astra session reviews the flows and implementation before publication. Work takes place in this shared checkout. Packages below define independent boundaries so implementation and review can proceed in parallel without conflicting edits.

**Status:** implementation packages A–H, integration onto `dev`, and fresh independent Astra review are complete. Approved for the user-requested draft PR and preview guide gist.

**Source:** [converged feature map and review](2026-11-28-agent-details-feature-map-and-ux-review.md). The user authorized planning, implementation, direct review and repeated corrections through completion on 2026-11-28.

## Outcome and constraints

Agent details should make it easy to edit the agent's working instructions, understand its permissions and approval behavior, connect its channels, and predict whether a control applies immediately or needs saving. Preserve the existing five-tab drawer and established design language.

- Reuse `GroupedList`, `GroupedRow`, `GroupedDetail`, grouped form/save/review components, Radix controls, existing typography, spacing, colors, borders, compact headers and viewport behavior. No new visual theme, dashboard of counts, arbitrary styling system or dependency.
- Preserve existing authorization, feature flags, role-specific visibility, read-only/destroyed states, machine isolation and capability restrictions, guest boundaries, billing rules, revocations, operation fences and credential secrecy. UI reorganization must never widen authority.
- Keep existing API field names and deep-link values (`details=grants` still addresses the visible Access tab). User-facing labels use Access. Keep exact proposal and channel-connection links working.
- Keep one authoritative editor per setting. Cross-links may reveal it. Retain drafts across tabs/search and the coordinated review-before-save path. Preserve keyboard navigation, focus restoration, accessible names, error/retry states and responsive layouts.
- Use `useAppForm`, existing hooks and domain schemas. Localize changed copy through the existing English, Simplified Chinese and Traditional Chinese catalogs. Preserve interpolation, glossary and i18n checks.
- Do not change repository/organization rulesets, branch protections, rollout flags, live settings or deployments. The user’s subsequent request authorizes a fresh Astra review, feature-branch commits/push, a draft PR and a new explanatory GitHub gist. It supersedes the initial implementation-only publication restriction.

## Intended information architecture

| Tab | First level | Nested or contextual detail |
| --- | --- | --- |
| Overview | Editable identity and purpose/instructions; compact ownership/role and effective approval/source context | Tone and style; call instructions with voice setup link; Sharing and Automations entries; lifecycle |
| Access | Approval behavior near the top; service/account and machine access groups | Granted operations, guests' allowed service actions, machine capabilities/logins, optional device voice |
| Skills | Attached skills and clear Find skills entry; pending learning proposals where available | Version selection, readability check, preloading, editing, history, learning settings and review |
| Memory | Remembered notes and visible privacy/scope explanation | Organization edits and shared-agent private/team notes |
| Channels | Connected bots/status and Connect/Register entry | Owner verification, audience and existing guest summary, replies/posts/threads, agent-wide progress reply, diagnostics |

Instructions define working behavior; Tone and style defines communication; Memory retains facts/preferences; Skills supplies reusable procedures. Keep specialist role/purpose explicit. Names of existing API fields and internal schemas remain compatible.

## Work packages

Use `todo`, `implementing`, `review`, `fixing`, `verified`. The primary reviewer updates final status after independently reviewing the resulting code and relevant validation. Implementer reports changed files and tests after each coherent package or batch.

| ID | Package | Primary files / boundary | Depends on | Status |
| --- | --- | --- | --- | --- |
| A | Approval and publication consequences | `nyxbot-labels.ts`, `agent-learning-review.tsx`, relevant help/copy/tests | none | verified |
| B | Save scope and immediate-action feedback | `ui/grouped-form.tsx`, immediate controls in touched panels | none | verified |
| C | Drawer organization and search | `nyxbot-agent-details.tsx`, `drawer-section-info.ts`, search metadata | A, B | verified |
| D | Skills and Memory clarity | skill/learning/preload and memory labels/help | A, C interface agreement | verified |
| E | Channels and guest navigation | `nyxbot-channels.tsx`, `bot-audience-settings.tsx`, channel registration integration | C navigation interface | verified |
| F | Voice and connection scope | `agent-voice-service.tsx`, `agent-connections.tsx`, Overview links | C navigation interface | verified |
| G | Sharing organization selection | `shared-agent-controls.tsx`, existing `useOrgs`/picker patterns | none | verified |
| H | Localization, integrated validation and direct review | affected catalogs/tests, browser fixtures and this record | all | verified |

Packages A/B/G can be developed independently. Packages D/E/F can follow in parallel once C's reveal/navigation interface is stable. Because the user requested one Heca implementer, this session uses those boundaries for sequential implementation while the primary reviewer inspects completed work and prepares independent validation. Avoid concurrent edits to the same file. The implementer owns product/test/catalog edits; the reviewer owns this plan and the findings record unless a correction is explicitly coordinated.

### A — Accurate action consequences

- Replace the absolute Full access promise (“Never asks. Every action…”) with accurate automatic-approval wording. Independent protected organization-connection requirements remain possible when their policy is activated. Keep descriptions consistent with chat controls and existing runtime behavior.
- Use Approval behavior or Ask before acting for the field's visible purpose; distinguish capabilities from approval timing. Show inherited source and effective value without changing inheritance semantics.
- Learned analysis creates pending proposals and can produce none. Enabling learning alone does not publish a learned proposal.
- The first explicit learned-publication request can publish and attach immediately when the approving chat is in Full access. That mode is resolved by the backend and is not necessarily the target specialist's mode. Never infer it solely from the displayed agent.
- Make labels/explanations correct for the known policy. If the client cannot know the approving mode, use an honest first-click label and explanation that allows immediate publication; do not promise a second confirmation. Retain the returned confirmation-card path and recovery/check/retry states.
- Authored proposals keep their own conversation-card flow. Do not merge learned and authored authority or change backend approval semantics merely to simplify copy.
- Rename Threshold to a unit-bearing explanation such as Analyze after N eligible conversations. Do not promise every analysis creates a proposal.

Acceptance: users can predict first-click publication behavior, understand Full access does not grant more service capabilities, and see correct review/recovery wording for each proposal state. Existing authorization and publication tests remain valid; add focused behavior coverage for revised branches where useful.

### B — Visible saving model

- Visibly expose the existing unsaved-section summary in the coordinated save bar, with wrapping/truncation appropriate to narrow drawers and retained live announcements. Avoid duplicate announcements.
- Preserve Save changes → Review changes → Save, and Discard across coordinated drafts. Keep save controls outside scroll content.
- Distinguish immediate mutations from drafts in the touched skill, learning, audience and channel editors. Use concise Applies immediately guidance where needed and existing local success/error/pending patterns; avoid toasts for every keystroke.
- Discard must never imply rollback of already-applied actions. Partial saves must identify successful/remaining sections through existing outcomes and visible pending state.
- Do not require a new confirmation or staged save for every simple action.

Acceptance: dirty sections are visible across tabs/search/nested pages; saving failure retains the appropriate draft; immediate controls give clear pending/result feedback and do not misleadingly join Discard.

### C — Organize the drawer without changing its visual language

- Keep the five tabs and compact/search/scroll structure. Keep display name and instructions directly editable in Overview.
- Break the long Profile card into meaningful existing-style groups: identity/purpose/instructions, tone/style, voice guidance as appropriate. Avoid adding a dashboard above them.
- Surface owner/role and effective approval/source in compact context using existing grouped rows. Reuse available agent data and settings; do not add expensive new aggregate queries for summary counts.
- Place the approval editor near the top of Access, in its own form/group with correct save registration. Show a linked effective summary in Overview.
- Place channel progress reply with Channels, explicitly agent-wide. Preserve a route/editor for eligible maintainers whose Channels tab is filtered out, including organization configurations without channel rollout/binding. Shared users and destroyed agents retain current limitations.
- Keep Sharing and Automations discoverable through concise entry rows; preserve existing destinations/lifecycle confirmations.
- Update section help and search metadata for Instructions, Tone and style, approval/permissions aliases and relocated controls. Search must reveal the real editor and focus it without discarding drafts.

Acceptance: role matrix remains intact, hidden-tab fallback works, incoming `grants`/skills/proposal/channel links work, only one editor per setting is mounted/registered as authoritative, and no drafts are reset by navigation.

### D — Skills and Memory clarity

- Provide a clear Find skills action/entry at the first level using current grouped controls. Retain exact version preview, revisions, removal, disabling and update behavior.
- Describe Test skill as checking access/readability, rather than task-quality execution; keep existing network behavior.
- Explain preloading in plain language, retaining path limits, guest exclusion, context cost and version-reset consequences.
- Add concise distinctions among Instructions, Memory and Skills at relevant entry points. Do not move private notes into shared memory or alter scope.
- Retain organization memory editing, shared-recipient private/published team separation, and destroyed read-only behavior.

Acceptance: labels describe actual behavior and existing skill/memory contracts remain intact.

### E — Channel setup and contextual guest access

- Add a working Register a bot action from the first-use/empty Channels view, using the existing registration route/dialog. Preserve selected agent context and make the return to connection setup concrete. Reuse existing routing/hosted flows rather than inventing a separate credential form.
- Distinguish registered, connected and verified states and provide the next relevant action.
- Replace visible Grants references with Access while retaining `details=grants` URLs.
- Preserve and improve the existing contextual audience/service/account/payer information. Organization Channels already exposes services, payer and wallet prerequisites; do not duplicate state or conflate shared-agent recipients, group participants and channel guests.
- Retain per-chat audience/posting/thread behavior, owner verification, reconnect/disconnect and diagnostics. Cross-links must honor draft guards and expected back behavior.

Acceptance: first-time registration can lead back to connecting a bot to the intended agent; guest access links target Access; existing readiness, membership, owner verification and billing gates are unchanged.

### F — Voice setup and connection scope

- Add contextual navigation between call instructions and optional Device voice setup. Keep ordinary chat independent of optional device integration.
- Explain service creation and Agent Key eligibility as distinct states. Keys already permitting all services can already include the service under their policy; do not universally require a key change or claim device readiness without evidence.
- Keep existing setup/revoke/manage actions and permission boundaries. Do not add raw key display or collection.
- Show All my agents scope beside connection reorder action; retain existing personal-before-organization rules and persisted ordering behavior.

Acceptance: voice setup/instructions are connected, service-created state does not overclaim device access, and connection reordering scope is visible before editing.

### G — Organization selection in Sharing

- Reuse current organization data and established Select/Combobox patterns to select organizations by display name/slug.
- Retain stable UUID values in the request and existing saved audience entries, including organizations not currently returned by the list. Do not drop or silently replace saved policies.
- Preserve person-by-email entry, roles, validation, 500-entry bound, explicit service sharing, private chats and payment semantics.
- Handle loading, empty, unavailable and error states. Do not introduce a new authorization grant or require admin membership where existing sharing semantics do not.

Acceptance: eligible organizations can be selected by name; existing audiences survive round trips; person sharing and all limits remain correct.

### H — Integrated review and verification

- Update affected `en`, `en-overrides`, `zh-CN`, `zh-TW` catalogs using repository tooling/patterns. Keep changes scoped and preserve glossary/interpolation.
- Run focused existing Vitest suites for Agent details/search, learning, channels/audience, skills, sharing, connections, voice and grouped save behavior as appropriate; extend tests for moved stateful controls and meaningful regression risks.
- Run frontend typecheck/build, ESLint, typography and i18n validation. Resolve failures introduced by these changes; investigate any baseline failures before declaring completion.
- Exercise the real fixture-backed drawer in a browser at compact and wider sizes, including search, keyboard navigation, dirty tab changes, nested Back, save review, read-only roles and relocated controls. Use local test fixtures, not live account mutations.
- The primary session reviews every changed product file and integration boundary itself, reports concrete findings to the Heca implementer, verifies corrections and repeats until no known in-scope findings remain at any severity.
- Document actual commands/results and review iterations below. Do not mark unfinished packages verified, omit known nits, or describe unperformed validation as passed.

## Review ledger

| Round | Scope | Findings | Resolution / evidence |
| --- | --- | --- | --- |
| Planning | Converged review → implementation packages | Preserve existing design and authority; keep five tabs; avoid count dashboard; accurate learned/approving-chat semantics; hidden Channels fallback | Captured above; ready for Heca implementation |
| Baseline | Existing implementation and tools | No starting test/type errors in focused scope; existing lint warnings recorded | 178 tests passed; typecheck and lint/typography/i18n passed before implementation |
| 1 | A/B and initial G changes | Correct main approval/publication changes. Requested accurate authored-policy aside, per-action learning result feedback, natural publication label, and nonduplicated organization labels | Closed: copy, success/error feedback and labels corrected; localized catalogs and regression coverage passed |
| 2 | C/F integration and A/B/G verification | Found Device voice row children suppressing its eligibility description; requested correct hidden-Channels fallback breadcrumb and readable extracted forms | Closed: eligibility content retained, breadcrumb follows placement, extracted forms formatted; primary 38-test intermediate check and final integrated checks passed |
| 3 | C/E navigation | Requested authoritative editor as first approval search result; identified organization registration scope mismatch that could return a newly created bot outside the target agent's eligible list | Summary indexing removed; contextual org scope locked; primary browser checks at 390/1280 confirm preserved grouped appearance, editable instructions, approval cross-link, retained profile draft and visible pinned save scope; regression and browser validation passed |
| 4 | C/F failure and asynchronous state paths | Requested authoritative voice readback to clear stale creation errors, truthful unknown workspace-mode display, ordinary modified-link behavior and retained progress drafts when org Channels visibility resolves asynchronously | Implementer added retained portal form, unknown-mode states, readback-scoped error handling and ordinary-click interception; regressions and primary final review passed |
| 5 | H localization/search review | Requested localized approval descriptions and static/dynamic help resolution. Reproduced a real helper failure for inherited object-property titles such as `toString` that can appear in arbitrary search metadata | Own-key lookup and localized static/dynamic resolution implemented; primary zh-CN/zh-TW runtime checks passed, including safe unknown metadata |
| 6 | Final consistency and registration integration | Requested consistent shared-profile heading after Persona → Tone and style rename; checked reusable registration flow and its intended next step | Closed: Shared profile heading and scope help aligned; primary verified registration, selected-agent connection, owner verification as next step and retained profile draft |

## Validation record

Baseline, before implementation:

- `npm ci --no-audit --no-fund`: completed; lockfiles unchanged.
- `npm test -- src/components/assistant/nyxbot-agent-details.test.tsx src/components/assistant/agent-learning-review.test.tsx src/components/assistant/agent-learning-review.request.test.tsx src/components/assistant/shared-agent-controls.test.tsx src/components/assistant/agent-connections.test.tsx src/components/assistant/agent-voice-service.test.tsx src/components/assistant/bot-audience-settings.test.tsx src/components/assistant/nyxbot-channels.test.tsx`: 8 files / 178 tests passed.
- `./node_modules/.bin/tsc -b --pretty false`: passed.
- `npm run lint`: passed, 0 errors / 34 existing ESLint warnings; typography and i18n guards passed. Most warnings concern Fast Refresh mixed exports, plus existing unrelated compiler/hook warnings. Touched-file warnings and any newly introduced warnings are reviewed during implementation.
- Installed Playwright Chromium verified; local fixture review server started on port 4612. Implementation/browser regression tests use the separate default port 4611.

During implementation, primary-review evidence:

- Reran sharing, grouped forms, learning component and learning request suites after the first corrections: 4 files / 38 tests passed.
- Fixture-backed Chromium at 390px and 1280px: Overview instructions remain visible; Approval behavior cross-link selects Access and focuses its editor; a profile draft survives navigation and the pinned bar visibly names it. Baseline and intermediate captures are in ignored `frontend/test-results/agent-details-review/`.
- Fixture-backed Chromium at 320px dark and 1280px dark: edit Instructions, select approval override, use search to reveal Channel progress, edit it, review all three independent drafts, keep editing and discard all three. Passed; pinned summary fits and no Vite error overlay. Captures are in the same ignored review-artifact directory.
- Fixture-backed Chromium at 390px dark: actual reusable bot registration with the built-in dummy-token API boundary returns to the intended agent's connection setup without navigation away or losing a profile draft. The new bot can be selected for connection; registration and verification remain distinct.
- Actual Simplified and Traditional Chinese drawer runtime at 390px dark: translated Instructions and section help, localized approval search opens Access, Full access consequence text is translated, dynamic Rule/Preloaded help resolves, and unknown object-property metadata is safe. Passed in both languages. Captures: `access-zh-CN-dark-390.png` and `access-zh-TW-dark-390.png` in the review-artifact directory.

Final validation (all passed):

- Implementer focused regression: **18 files / 308 tests**, including moved independent forms, partial save/Discard, delayed channel placement, unknown inherited approval, publication protocol, skill and voice behavior, organization selection, registration, localized help and safe arbitrary search titles.
- Primary independent final regression: **2 files / 77 tests** (`nyxbot-agent-details.test.tsx`, `drawer-section-info.test.tsx`), including the final partial-save regression.
- Chromium: **7 / 7 cases** across the new drawer suite, existing connections suite and updated profile persistence case. Covers 390px/1280px, keyboard search, nested Back/focus, review/Keep editing/save/reload, registration/cancel/return/connection, and immediate connection changes surviving draft Discard.
- Primary `npm run build`: passed. Includes TypeScript, application production bundle, legal prerender, credential-accept bundle and mock-footprint assertion.
- Primary `npm run lint`: passed, 0 errors / 33 existing warnings (baseline 34; removed the touched drawer helper-export warning). Typography and i18n guards passed.
- Primary `git diff --check` and local links in both Markdown documents: passed.
- Primary final browser run: newly registered demo bot connected to the intended NyxBot; owner verification remained the next step; an unsaved profile draft survived the entire flow.

Reproduce the focused run from `frontend/`:

```sh
npm test -- src/components/assistant/nyxbot-agent-details.test.tsx src/components/assistant/agent-learning-review.test.tsx src/components/assistant/agent-learning-review.request.test.tsx src/components/assistant/nyxbot-channels.test.tsx src/components/assistant/shared-agent-controls.test.tsx src/components/assistant/agent-voice-service.test.tsx src/components/assistant/agent-connections.test.tsx src/components/assistant/agent-skills.test.tsx src/components/assistant/bot-audience-settings.test.tsx src/components/channels/channel-bot-setup-errors.test.tsx src/components/ui/grouped-form.test.tsx src/components/ui/section-info.test.tsx src/lib/drawer-section-info.test.tsx src/lib/assistant/nyxbot-labels.test.ts src/lib/assistant/nyxagent-http-fixtures.test.ts src/lib/drawer-search.test.tsx src/hooks/use-agent-learning-review.test.tsx src/lib/nyxbot-channels.test.ts
```

Reproduce the Chromium run from `frontend/`:

```sh
./node_modules/.bin/playwright test e2e/agent-details.spec.ts e2e/agent-connections.spec.ts e2e/nyxagent.spec.ts --grep 'Agent details retains|registration returns|Agent Details groups|Add context keeps|agents get a display name and persona'
```

Final primary captures, retained in the ignored test output directory after automated runs completed:

- Overview, dark, 390px: local-only capture `final-overview-dark-390.png`.
- Access, dark, 390px: local-only capture `final-access-dark-390.png`.
- Registered bot connected, dark, 390px: local-only capture `final-registration-connected-dark-390.png`.

The implementation uses the existing grouped components, typography, theme tokens and five-tab shell. API field names, deep-link values, authority/role restrictions and feature flags retain their existing contracts. The initial implementation passed the direct-review checks above. The fresh review and validation after integration onto current `dev` are recorded below.

## Fresh review and publication

The user subsequently requested a new Astra review of both flow and implementation, a draft PR after the review is clear, and a new GitHub gist describing the changes and organization. The reviewer is a fresh Heca `gpt-6.2-astra` session at `xhigh`. The checkout was 518 commits behind `origin/dev` at inspection; publication requires integrating the feature onto the current base, preserving upstream additions, rebuilding the embedded CLI wizard when its shared source closure changes, and rerunning affected checks. Review findings, resolutions, final base/commit and publication links will be recorded here.


### Fresh review closure

The feature was rebased onto `dev` at `fca745e13`, preserving the upstream avatar editor, built-in skills, organization governance, guest preflight and setup-completion callback. The repository’s official i18n adoption tool moved the feature’s entries to the split app catalogs.

The fresh Heca `gpt-6.2-astra` / `xhigh` reviewer initially requested changes and approved the corrected flow, implementation and change guide after re-review. The reviewer used local fixtures and reported no remaining actionable findings.

| Review finding | Resolution and evidence |
| --- | --- |
| Managed Telegram resumed a personal or other-organization request inside locked organization registration. | Immutable expected owner reaches the parent and action component. Mismatched saved requests and responses cannot change scope, launch, cancel or complete the contextual flow. Separate recovery and platform choice remain available. Matching/mismatched owner regressions and independent review passed. |
| A non-maintainer organization member’s audience link had no registered Access target. | The readable grants branch registers the same navigation target; ordinary and modified click behavior is preserved. |
| Learning could show an unaccepted threshold, write on unchanged blur, or overwrite unknown settings. | Read readiness and Retry, integer validation, off-state disabling, unchanged-write suppression, rejected-value restoration and authoritative concurrent readback. UI and hook regressions passed. |
| Channels and approval descriptions overclaimed immediate settings or prompt-free reads. | Immediate channel operations are named; staged progress uses Save changes. Independent protected organization read policy is explicit. |
| Chinese review labels, search prompts, fallback breadcrumbs and deeper fixed metadata were partly English. | Static producers and fallbacks are localized, including Agent memory; user titles, service names and note content remain verbatim. Runtime tests cover both Chinese locales. |
| Section help rebuilt its full localized reverse map on every search lookup. | Canonical-key fast path and a finite cached reverse map invalidated on language/catalog changes. Arbitrary user titles are not cached. |
| Skill feedback followed unrelated pins, removal retained stale detail, and Find skills lost input focus. | Feedback identifies its affected pin; removal returns to the list with focus; catalog search is the final navigation focus target. Unit and browser regressions passed. |
| Eligibility/optional voice/progress timing guidance was imprecise. | Empty states reflect eligibility and role. Voice help is conditional; progress copy/spec measures five seconds from turn start with external work observed. |
| Org demo registration lost its owner and the preview lacked current read contracts. | Demo owner is retained. Preview provides grant inventory, saved-logins, preference/trigger reads and scoped CMA origin pages on the actual consumer boundary. The real CMA consumer validates response shape, zero fetch and abort behavior. |
| Screenshot links pointed into ignored test output and memory guidance overgeneralized immediacy. | Six stable PNG assets are committed under `docs/assets/agent-details`. Guide/spec distinguish staged memory Add/Edit from immediate Forget/Delete and provide a reproducible fresh-draft walkthrough. |

Final primary verification:

- Focused integration run: **25 files / 487 tests passed**. This extends the original command above with settings-content, current avatar/governance coverage, managed Telegram parent/child/resume, change descriptions and mock data.
- Final transport/preview correction run: **3 files / 88 tests passed**: `cma-session-api.test.ts`, `nyxagent-http-fixtures.test.ts`, `mock-data.test.ts`.
- Chromium: **8/8 passed** at 390px/1280px, including a permanent regression proving Find skills accepts keyboard input after deferred navigation focus settles. The display-name/persona test now scopes the upstream duplicate entry points and waits for the review modal to finish closing before interacting with the sheet.
- Production build: TypeScript, Vite, credential acceptance, mock-footprint and the **92-page** public artifact gate passed.
- Full lint: **0 errors**, with 58 warnings in unchanged files on the integrated base; typography and i18n checks passed. No touched-file warning remains.
- Embedded CLI wizard rebuilt; the generated HTML, 216-module manifest and source hash are consistent. A separate SHA-256 readback using the exact committed freshness algorithm passed. Cargo is unavailable locally; the Rust freshness wrapper is left to the existing CI job.
- `git diff --check` passed. Six captures were reviewed visually after transitions settled.

Fresh independent evidence is separate from the primary checks: the reviewer passed **324 tests / 13 files**, then **88 tests / 3 transport/preview files**, and exercised 390px Chinese search/review/focus and 1280px registration/connection/Discard flows. Its final verdict is **APPROVE**.

Final browser command from `frontend/`:

```sh
./node_modules/.bin/playwright test e2e/agent-details.spec.ts e2e/agent-connections.spec.ts e2e/nyxagent.spec.ts --grep 'Agent details retains|registration returns|Agent Details groups|Add context keeps|agents get a display name and persona|Find skills keeps'
```

The [change guide](2026-11-28-agent-details-change-guide.md) contains the current feature map, access improvements, save/action semantics, role behavior, walkthrough, local preview command and stable captures. The draft PR description links the standalone unlisted gist and revision-pinned preview assets. Publication is limited to the feature branch, draft PR and gist; no merge, deployment, live configuration or protection changes are part of this work.

# Native workspace routes

The signed-in workspace has a base sidebar and one main content slab. Opening
navigation translates the slab right by 77% of the window width; no scrim or
panel covers it. The near-black dark sidebar and gray light sidebar sit behind
the elevated content surface. The slab keeps its rounded left corners and shadow.

Only Settings, its account pages and the QR/terminal quick actions use the
pull-up surface. The full-screen desktop viewer retains its own presentation.
Other task-local pickers, confirmations, attachment preview and voice call are
transient interactions, rather than workspace destinations.

## Route inventory

`navigationTypes.ts` is the authoritative registration list. All routes in
`WorkspaceContentParamList` render in the slab; nested pages push horizontally.

| Stack / routes                                                                       | Purpose                                                                                                                                    |
| ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Content / Assistant                                                                  | Explicit NyxBot Home or a private chat. `home: true` selects Home; `home: false` and a fresh `draftId` select an empty normal chat.        |
| Content / AssistantDetails, AssistantHistory, AssistantWorkingContext                | Readable chat details with guarded rename/delete, agent-scoped paged history, checkpoint and guarded context reset.                        |
| Content / AssistantContext, GroupChat                                                | Contextual private and crew chats, preserving the primary chat and draft.                                                                  |
| Content / Automations, AutomationDetail, AutomationRuns                              | Read-only configured tasks, details and cursor-paged run history, with exact conversation links.                                           |
| Content / Machines, MachineDetail, MachinePolicy, MachineActivity                    | Inventory, detail, owner policy, updates, saved logins and machine activity.                                                               |
| Content / AgentMachines, AgentMachineEditor                                          | Guarded per-agent machine access.                                                                                                          |
| Content / Plugins, PluginConnections, Services, ServiceSetup                         | Plugins, connections and hosted service setup.                                                                                             |
| Content / Agents, AgentCreate, AgentDetail, GroupEditor                              | Agents, crews and their guarded editors.                                                                                                   |
| Content / AgentSkills, SkillDetail, SkillVersions, SkillHistory, AuthoredSkillReview | Read-only skill files/history, exact version pinning and owner publication decisions.                                                      |
| Content / Activity, ActivityDetail, ActiveGrants                                     | Incoming actionable approvals, date-grouped history, detail and the existing grants view. Canonical approval links still address Activity. |
| Content / WorkspaceSearch, WorkspaceRecents                                          | Grouped local search and cursor-paged See all, with chat-title search and per-agent filtering.                                             |
| Sheet / AccountSettings, AccountSecurity, UsageCredits                               | Settings, account/security, display, notifications and usage/credits.                                                                      |
| Sheet / DeviceLogin, AgentKeyLogin                                                   | QR, terminal and Agent Key sign-in flows with existing write guards.                                                                       |
| Sheet / TermsOfService, PrivacyPolicy                                                | Signed-in legal documents.                                                                                                                 |
| Root / Workspace, WorkspaceSheet                                                     | Main signed-in shell and the account/quick-action sheet host above it.                                                                     |
| Root / MachineDesktop                                                                | Full-screen native desktop; landscape while focused/active, upright portrait restored on exit/background. No desktop web fallback.          |
| Root / Auth, DeviceLogin, AgentKeyLogin, TermsOfService, PrivacyPolicy               | Signed-out authentication and shared canonical entries; signed-in wrappers resolve into the account sheet.                                 |

Chat Details, History and Working context are nested content pages. History
reuses the thread browser, initially filtered to the viewed agent; its New chat
keeps that agent. Choosing Details/History from the agent picker first dismisses
the native picker, then pushes the content page. Agent/permissions/options and
recipients/connections pickers remain transient choices. Attachment preview and
voice conversation retain their media presentation.

## Sidebar and destination transitions

The sidebar header contains the NyxID logo and circular Search control. Quick
actions (Scan QR code and Terminal login code) precede Workspace. Workspace
contains Automations, Machines, Plugins, Activity with its live pending count,
and Create new agent. There are no disabled coming-soon destinations. Agents
(personal, then organization), Crews and plain-text Recents follow. The pinned
Chat pill opens NyxBot Home; the gear opens Settings. A measured bottom spacer
keeps the last row above the bar at maximum scroll. A fade hides rows passing
behind it during scrolling.

The logo and bottom Chat pill select Home. Bare `/assistant` and a cold start
without a restorable chat also select Home. Header compose and the agent
picker's New chat retain the viewed agent and create an empty draft, including
NyxBot. Home never substitutes for a new chat. Crew headers preserve their
existing conversation/recipient semantics rather than exposing private compose.

A sidebar destination first fades out the old root in 120ms, then swaps it with
native-stack animation `none`. After the new page has laid out and painted,
it fades in over 180ms while the slab closes once over 300ms. Programmatic
transitions use timing; finger release uses a clamped spring. Reduced motion
skips travel and fades. Rapid selection is fenced by transition identity.

Closed root pages reveal navigation only from a 24pt left-edge zone with a
rightward threshold and vertical failure range. Nested pages keep back-swipe.
When open, only the visible slab sliver accepts a closing tap/drag. Composer,
transcript and horizontal table/code scrolling do not start the sidebar pan.

## Sheets, guards and accessibility

Settings and quick actions animate over the current sidebar/slab state. Opening
or dismissing them preserves the expanded offset and sidebar scroll position.
Their host sits above the shell; the shell is pointer- and accessibility-hidden
while the sheet is presented. Android Back dismisses the sheet first. A sheet
owner waits for dismissal animation before unmounting. Reduced motion is instant.
A main-content link chosen from a sheet dismisses it before selecting its page.

A sheet shows Back only after an internal push, based on its local stack index.
Settings root, direct QR/Terminal entry and cold account links have Close only.
Signed-in page/sidebar headers share a 60pt row below the safe area. Main-page
and Settings titles reserve equal control columns, center across the full width
and cap scaling at 1.25×. Chat identity retains its avatar/name layout.

The presentation-independent leave guard covers dirty/critical writes through
Back, sidebar destination selection, edge gestures and Android Back. A pending
critical write blocks dismissal; a dirty editor asks before discard. Settings
retains its equivalent Close/backdrop/drag/Back/Escape guards. Sign-out clears
account state and unmounts editors. `navigationHierarchy.test.ts`,
`workspaceChatNavigation.test.ts` and the surface/guard tests cover route,
restoration, transition and dismissal behavior.

Accessibility exposure follows the shell's settled visual endpoint. While
navigation is open the slab is hidden except for Close navigation; focus moves
to the sidebar and restores to the current content control on close. During
travel neither stale sidebar targets nor covered content are exposed.

## Canonical links and native scope

Canonical URLs remain stable. Content links resolve to ContentStack; cold links
have Workspace/Home as the base. Settings/account links resolve to the sheet.
Existing chat IDs, route keys and account-scoped drafts survive navigation.
Current-thread reselection preserves unsent text; new chat never transfers it.

Native skills are read-only by user decision 2026-10-28. Choosing a version pins
its exact version and hash; Use latest resolves the newest non-deprecated
package and pins it immediately. Remove from agent removes only that agent's
pin. Owner approve/decline cards remain; native draft/editor/publish-from-draft
routes are deleted. To change skill content, ask NyxBot.

Native Automations are read-only by the same user decision. Editor, creation,
preferences, invoke, toggle, delete and webhook setup paths are removed. The
feature exposes GET only. Old authoring links resolve to the configured list or
exact read-only detail. Guidance says “To create, run or change automations,
ask NyxBot.” Its action opens an empty normal chat without sending text.

Permission modes remain editable subject to the existing owner/agent-override
contract. Chat and agent inference configuration is server-owned; native has no
configuration picker, conversation configuration PATCH or new-turn override.
Voice automatically selects the available server default without a selector.

# NyxID Chat - Four Target-State Use Cases

These transcripts depict the target-state flow in section 4 of the NyxID Chat - Aevatar
Support Contract: gaps G1-G9 are assumed closed except where the contract itself reserves a
capability, leaves a bridge unspecified, or leaves a v1 scope decision open (G9). The `web`
executor remains reserved and is never emitted. For lane-A NyxID approvals, only observable
behavior is shown: NyxID records the decision, and Aevatar wakes and rechecks the gated step.
The G8 bridge remains unspecified; no bridge command is invented.

All people, repositories, invoices, candidates, results, links, and identifiers below are
fictional. The user is Calvin, acting in the same fictional organization throughout. UC1a and
UC1b are alternative starting states, not sequential tasks: UC1a begins with GitHub and Lark
unconnected, while UC1b begins with both connected. UC3 and UC4 independently assume that Lark
(`lark-feishu`) is connected.

## Transcript convention

`User` and `Assistant` messages are blockquotes. Italic square-bracketed narrator lines report
facts outside the chat surface. Cards use the following closed set:

- `CARD: INPUT` renders an actor-owned pending input from `nyxid.input.request`; the answer is
  one `input.resolve` command containing either one `freeText` value or one flat list of opaque
  option IDs.
- `CARD: PLAN` renders the full `TaskPlan` from `nyxid.task.snapshot`.
- `CARD: CONNECT` renders a NyxID browser action from `nyxid.action.request`; completion wakes
  the actor with `action.continue` and is proven by a typed postcondition.
- `CARD: NYXID APPROVAL` renders a lane-A NyxID-owned decision surface. Aevatar observes the
  outcome but cannot decide, loosen, or bypass it.
- `CARD: ARTIFACT` is a documentation-only visual treatment of the assistant's final delivery
  message. It is not a wire frame, command, or contract object.

Status glyphs map one-to-one to the closed step statuses: `○ planned`, `⏸ waiting`, `◐ running`,
`● done`, `✕ failed`, `⊘ skipped`, `⊗ cancelled`, and `? uncertain`. Every plan row spells out
the literal status and renders `externalEffect` separately. Substeps use only `running`, `done`,
or `failed`; they are presentation phases within one operation and never hide another external
call.

Readable IDs follow one disjoint scheme: tasks such as `task-uc1a`; origin and continuation
turns such as `turn-uc1a-1` and `turn-uc1a-2`; browser actions such as `act-github-1`; inputs
such as `input-uc1a-gaps` and `input-uc1a-gate`; and NyxID approvals such as
`nyxid-appr-uc1a-1`. One goal keeps one task ID across its turns, and every continuation gets
a new turn ID.

Each main task starts at plan revision 1 with only its gap-input step. After one composite gap
answer, disclosed capability-resolution reads may run, and revision 2 communicates the full
execution approach. In a confirm-gated plan, the gate is the first pending step of this
execution-phase revision; the already-done gap step remains visible ahead of it. This is the
explicit interpretation of section 7.5's "first step" rule used here. Gap and gate inputs never
share a `requestId`. Lane B in these transcripts is demonstrated by gap and plan-gate inputs;
an Aevatar-scoped tool approval is intentionally out of scope, so `approval.resolve` does not
appear in any scenario.

`source: tool: X` is documentation shorthand for the step's tool identity: slug-derived for a
NyxID-brokered operation and the mounted skill identity otherwise. It does not replace section
7.3's structured `source.kind: "tool"` fields. Consecutive `CARD: PLAN` snapshots represent one
live plan card re-rendering in place; they are shown separately here only to make state changes
auditable.

## UC1 - GitHub digest variants

### UC1a - GitHub not connected

*Precondition: `api-github` and `lark-feishu` are both in the catalog but neither appears in
Calvin's connected-service inventory.*

**User**
> Summarise this week's merged PRs and post the update to #eng-updates.

*(wire: `text` creates `turn-uc1a-1` and `task-uc1a`; plan revision 1 begins.)*

**Assistant**
> I need three choices before I can propose the approach. Please answer them together: which
> repositories should I include, what date window should "this week" mean (I suggest Monday,
> 3 August 2026 through today), and should the summary be plain or grouped by area?

```text
CARD: INPUT · request input-uc1a-gaps · answer all scoping questions once
 Prompt: Repositories, date window, and summary grouping?
 Answer mode: one composite free-text answer · allowFreeText: true · multiSelect: false
 Step status: waiting
```

*(wire: `nyxid.input.request` · requestId=input-uc1a-gaps · turnId=turn-uc1a-1 · taskId=task-uc1a · stepId=step-uc1a-gaps; revision 1 contains only this input step, and nothing has executed.)*

**User**
> Use nyxid-backend and nyxid-frontend, Monday 3 August through today, grouped by area.

*(wire: `input.resolve` · requestId=input-uc1a-gaps · answer={freeText: "nyxid-backend and nyxid-frontend, Monday 3 August through today, grouped by area"}.)*

**Assistant**
> I checked your connected-service inventory and readiness: GitHub is missing, and Lark is
> also not connected. Here's how I'll approach it before anything executes: NyxID will connect
> GitHub first and Lark second, one blocked turn at a time. Then GitHub will read merged PRs
> once per repository, I will draft a grouped update, Lark will post it after NyxID's approval
> gate, and a Lark read will verify the message. The gate is derived as `confirm` because the
> plan publishes outside NyxID.

```text
CARD: PLAN · task task-uc1a · revision 2 · gate: confirm - "connect GitHub and Lark first; posting publishes outside NyxID"
 1 ● done      You - answer scoping questions · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ⏸ waiting   You - approve this plan · source: input · addedBy: replan · status: waiting · externalEffect: not_started · availableActions: [stop]
 3 ○ planned   NyxID - connect GitHub · source: action: service.connect · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 4 ○ planned   NyxID - connect Lark · source: action: service.connect · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 5 ○ planned   GitHub - read merged PRs: nyxid-backend · source: tool: api-github__list_pull_requests · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 6 ○ planned   GitHub - read merged PRs: nyxid-frontend · source: tool: api-github__list_pull_requests · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 7 ○ planned   Assistant - draft the update · source: llm · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 8 ○ planned   Lark - post to #eng-updates · source: tool: lark-feishu__send_message · NyxID approval gate · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 9 ○ planned   Verify - confirm the post exists · source: postcondition: lark-feishu__get_message · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
```

```text
CARD: INPUT · request input-uc1a-gate · approve the communicated plan
 Option: Proceed (optionId: proceed)
 Free text: objections are folded into a re-plan
 Step status: waiting
```

*(wire: the disclosed Class-R inventory/readiness read precedes `nyxid.task.snapshot`; planRevision advances 1 -> 2, and every new execution step has addedBy=replan. GitHub and Lark connect steps precede all execution; sequential connect cards are the honest v1 G9 behavior.)*

**User**
> Proceed.

*(wire: `input.resolve` · requestId=input-uc1a-gate · answer={selectedOptionIds:["proceed"]}; this requestId is distinct from input-uc1a-gaps.)*

```text
CARD: CONNECT · action act-github-1 · GitHub
 Service: api-github
 Requested scopes: repo
 Step status: waiting
 Control: Connect via NyxID
```

*(wire: `nyxid.action.request` · action=service.connect · actionRequestId=act-github-1 · originTurnId=turn-uc1a-1 -> task/turn committed blocked; stream ends RUN_FINISHED(blocked).)*

*[narrator: Calvin completes the GitHub OAuth journey inside the NyxID popup. The secret lands
only in NyxID and is never displayed in the chat.]*

```text
CARD: CONNECT · action act-github-1 · GitHub
 Service: api-github
 Disposition: completed
 Postcondition: pending
```

*(wire: `action.continue` · originTurnId=turn-uc1a-1 · actionRequestId=act-github-1 · disposition=completed · resource={userService:{userServiceId:"user-service-github-uc1a"}} starts NEW turn turn-uc1a-2 on task-uc1a.)*

**Assistant**
> GitHub is connected and verified as executable. The next planned missing capability is Lark,
> so NyxID is presenting that connection now; you do not need to ask again.

*(wire: completed disposition was only a signal; `nyx__list_connected_services` matched slug=api-github and executable=true before step-uc1a-connect-github became status=done with externalEffect=confirmed.)*

```text
CARD: CONNECT · action act-lark-1 · Lark
 Service: lark-feishu
 Requested scopes: message:write, message:read
 Step status: waiting
 Control: Connect via NyxID
```

*(wire: `nyxid.action.request` · action=service.connect · actionRequestId=act-lark-1 · originTurnId=turn-uc1a-2 -> task/turn committed blocked; stream ends RUN_FINISHED(blocked).)*

*[narrator: Calvin completes the Lark OAuth journey inside the NyxID popup. The secret lands
only in NyxID and is never displayed in the chat.]*

```text
CARD: CONNECT · action act-lark-1 · Lark
 Service: lark-feishu
 Disposition: completed
 Postcondition: pending
```

*(wire: `action.continue` · originTurnId=turn-uc1a-2 · actionRequestId=act-lark-1 · disposition=completed · resource={userService:{userServiceId:"user-service-lark-uc1a"}} starts NEW turn turn-uc1a-3 on task-uc1a.)*

**Assistant**
> Lark is connected and verified as executable. Both planned connections are ready, so I am
> proceeding from the first GitHub read without asking again.

*(wire: completed disposition was only a signal; `nyx__list_connected_services` matched slug=lark-feishu and executable=true before step-uc1a-connect-lark became status=done with externalEffect=confirmed.)*

*(wire: task-uc1a resumes on turn-uc1a-3; steps 5 and 6 invoke `api-github__list_pull_requests` independently, one call per repository, and both complete before the draft. No substep carries an external operation.)*

**Assistant**
> Draft ready:
>
> **Platform** - Added delegation-token refresh safeguards and clarified uncertain proxy
> outcomes. **Auth** - Hardened device-code delivery and approval expiry handling. **Console**
> - Shipped the integrity status view and improved service readiness labels. Five PRs merged
> across the two repositories from 3 August through today.

```text
CARD: NYXID APPROVAL · nyxid request nyxid-appr-uc1a-1 · awaiting decision
 Badge: Decided on NyxID - not by the assistant
 Action: Post the prepared weekly digest
 Target: Lark #eng-updates
 Actor: Calvin via NyxID Chat
 Reversibility: reversible
 grantBoundary: nyxid_step_up
 Countdown: 09:42 remaining
 Other decision surfaces: NyxID mobile, Telegram, dashboard
```

**Assistant**
> This is NyxID's approval gate, not mine. I cannot decide, loosen, or bypass it. The first
> decision on any linked NyxID surface wins.

*[narrator: Calvin selects Approve on the embedded NyxID card. NyxID records the decision.]*

*(wire: the pending approval carries nyxidRequestId=nyxid-appr-uc1a-1; Aevatar only observes the NyxID outcome, then wakes and rechecks the gated step. G8's internal resume bridge is intentionally unspecified.)*

```text
CARD: PLAN · task task-uc1a · revision 2 · gate: confirm - "connect GitHub and Lark first; posting publishes outside NyxID"
 1 ● done      You - answer scoping questions · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      You - approve this plan · source: input · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ● done      NyxID - connect GitHub · source: action: service.connect · addedBy: replan · status: done · externalEffect: confirmed · availableActions: []
 4 ● done      NyxID - connect Lark · source: action: service.connect · addedBy: replan · status: done · externalEffect: confirmed · availableActions: []
 5 ● done      GitHub - read merged PRs: nyxid-backend · source: tool: api-github__list_pull_requests · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 6 ● done      GitHub - read merged PRs: nyxid-frontend · source: tool: api-github__list_pull_requests · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 7 ● done      Assistant - draft the update · source: llm · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 8 ● done      Lark - post to #eng-updates · source: tool: lark-feishu__send_message · NyxID approval gate · addedBy: replan · status: done · externalEffect: confirmed · availableActions: []
 9 ● done      Verify - confirm the post exists · source: postcondition: lark-feishu__get_message · addedBy: replan · status: done · externalEffect: confirmed · availableActions: []
```

*(wire: `lark-feishu__get_message` read the posted message back before task success; RUN_FINISHED reports completed only after this typed postcondition.)*

**Assistant**
> The update is posted and verified.

```text
CARD: ARTIFACT · weekly engineering digest - connected during task
 Destination: Lark #eng-updates
 Window: 3-5 August 2026
 Repositories: nyxid-backend, nyxid-frontend
 Merged PRs: 5
 Message: Platform, Auth, and Console summary shown above
 Link: lark.example/messages/eng-updates-2026-08-05-uc1a
 Verified against #eng-updates: the message exists and matches the prepared draft.
```

### UC1b - GitHub and Lark already connected

*Precondition: `api-github` and `lark-feishu` are already connected and executable for Calvin.*

**User**
> Summarise this week's merged PRs and post the update to #eng-updates.

*(wire: `text` creates `turn-uc1b-1` and `task-uc1b`; plan revision 1 begins.)*

**Assistant**
> I need three choices before I can propose the approach. Please answer them together: which
> repositories should I include, what date window should "this week" mean (I suggest Monday,
> 3 August 2026 through today), and should the summary be plain or grouped by area?

```text
CARD: INPUT · request input-uc1b-gaps · answer all scoping questions once
 Prompt: Repositories, date window, and summary grouping?
 Answer mode: one composite free-text answer · allowFreeText: true · multiSelect: false
 Step status: waiting
```

*(wire: `nyxid.input.request` · requestId=input-uc1b-gaps · turnId=turn-uc1b-1 · taskId=task-uc1b · stepId=step-uc1b-gaps; revision 1 contains only this input step, and nothing has executed.)*

**User**
> Use nyxid-backend and nyxid-frontend, Monday 3 August through today, grouped by area.

*(wire: `input.resolve` · requestId=input-uc1b-gaps · answer={freeText: "nyxid-backend and nyxid-frontend, Monday 3 August through today, grouped by area"}.)*

**Assistant**
> I checked your connected-service inventory and readiness: GitHub and Lark are both present
> and executable. I will use one GitHub read per repository, draft the grouped update, send it
> to Lark after NyxID's approval gate, and read the message back to verify it. This plan is
> using your existing GitHub + Lark connections — no new permission needed, so there is no
> connect card or fresh service-access click. The gate still derives as `confirm` because the
> Lark step publishes outside NyxID; existing access never bypasses that approval.

```text
CARD: PLAN · task task-uc1b · revision 2 · gate: confirm - "using your existing GitHub + Lark connections — no new permission needed; posting still requires approval"
 1 ● done      You - answer scoping questions · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ⏸ waiting   You - approve this plan · source: input · addedBy: replan · status: waiting · externalEffect: not_started · availableActions: [stop]
 3 ○ planned   GitHub - read merged PRs: nyxid-backend · source: tool: api-github__list_pull_requests · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 4 ○ planned   GitHub - read merged PRs: nyxid-frontend · source: tool: api-github__list_pull_requests · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 5 ○ planned   Assistant - draft the update · source: llm · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 6 ○ planned   Lark - post to #eng-updates · source: tool: lark-feishu__send_message · NyxID approval gate · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 7 ○ planned   Verify - confirm the post exists · source: postcondition: lark-feishu__get_message · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
```

```text
CARD: INPUT · request input-uc1b-gate · approve the communicated plan
 Option: Proceed (optionId: proceed)
 Free text: objections are folded into a re-plan
 Step status: waiting
```

*(wire: the disclosed Class-R `nyx__list_connected_services` inventory/readiness read is the only pre-communicate operation; planRevision advances 1 -> 2. Both services matched executable=true, so no `nyxid.action.request` is raised.)*

**User**
> Proceed.

*(wire: `input.resolve` · requestId=input-uc1b-gate · answer={selectedOptionIds:["proceed"]}; this requestId is distinct from input-uc1b-gaps.)*

```text
CARD: PLAN · task task-uc1b · revision 2 · gate: confirm - "using your existing GitHub + Lark connections — no new permission needed; posting still requires approval"
 1 ● done      You - answer scoping questions · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      You - approve this plan · source: input · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ◐ running   GitHub - read merged PRs: nyxid-backend · source: tool: api-github__list_pull_requests · addedBy: replan · status: running · externalEffect: not_started · availableActions: [stop]
 4 ○ planned   GitHub - read merged PRs: nyxid-frontend · source: tool: api-github__list_pull_requests · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 5 ○ planned   Assistant - draft the update · source: llm · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 6 ○ planned   Lark - post to #eng-updates · source: tool: lark-feishu__send_message · NyxID approval gate · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 7 ○ planned   Verify - confirm the post exists · source: postcondition: lark-feishu__get_message · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
```

*(wire: the two repository reads are two independent tool steps and two calls. Existing connections remove only connection friction; no read is approval-gated.)*

**Assistant**
> Draft ready:
>
> **Platform** - Added delegation-token refresh safeguards and clarified uncertain proxy
> outcomes. **Auth** - Hardened device-code delivery and approval expiry handling. **Console**
> - Shipped the integrity status view and improved service readiness labels. Five PRs merged
> across the two repositories from 3 August through today.

```text
CARD: NYXID APPROVAL · nyxid request nyxid-appr-uc1b-1 · awaiting decision
 Badge: Decided on NyxID - not by the assistant
 Action: Post the prepared weekly digest using the existing Lark connection
 Target: Lark #eng-updates
 Actor: Calvin via NyxID Chat
 Reversibility: reversible
 grantBoundary: nyxid_step_up
 Countdown: 09:47 remaining
 Other decision surfaces: NyxID mobile, Telegram, dashboard
```

**Assistant**
> GitHub and Lark needed no connection click, but this publish action still belongs to NyxID's
> approval gate. I cannot decide, loosen, or bypass it; the first linked NyxID surface to
> decide wins.

*[narrator: Calvin selects Approve on the embedded NyxID card. NyxID records the decision.]*

*(wire: the pending approval carries nyxidRequestId=nyxid-appr-uc1b-1; Aevatar only observes the NyxID outcome, then wakes and rechecks the gated step. G8's internal resume bridge is intentionally unspecified.)*

```text
CARD: PLAN · task task-uc1b · revision 2 · gate: confirm - "using your existing GitHub + Lark connections — no new permission needed; posting still requires approval"
 1 ● done      You - answer scoping questions · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      You - approve this plan · source: input · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ● done      GitHub - read merged PRs: nyxid-backend · source: tool: api-github__list_pull_requests · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 4 ● done      GitHub - read merged PRs: nyxid-frontend · source: tool: api-github__list_pull_requests · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 5 ● done      Assistant - draft the update · source: llm · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 6 ● done      Lark - post to #eng-updates · source: tool: lark-feishu__send_message · NyxID approval gate · addedBy: replan · status: done · externalEffect: confirmed · availableActions: []
 7 ● done      Verify - confirm the post exists · source: postcondition: lark-feishu__get_message · addedBy: replan · status: done · externalEffect: confirmed · availableActions: []
```

*(wire: `lark-feishu__get_message` read the posted message back before task success; RUN_FINISHED reports completed only after this typed postcondition.)*

**Assistant**
> The update is posted and verified.

```text
CARD: ARTIFACT · weekly engineering digest - existing-connections path
 Destination: Lark #eng-updates
 Window: 3-5 August 2026
 Repositories: nyxid-backend, nyxid-frontend
 Merged PRs: 5
 Message: Platform, Auth, and Console summary shown above
 Link: lark.example/messages/eng-updates-2026-08-05-uc1b
 Verified against #eng-updates: the message exists and matches the prepared draft.
```

## UC2 - Dinner reservation research, steering, and stop

*Precondition: no connected or catalog service can place a restaurant booking. A concrete
Aevatar ecosystem web-search skill is mounted.*

**User**
> Book a dinner reservation for the team on Friday - Greek food, somewhere in the north of
> Singapore, 6-7 pm.

*(wire: `text` creates `turn-uc2-1` and `task-uc2`; plan revision 1 begins.)*

**Assistant**
> I need one combined answer before I propose anything: party size, dietary restrictions,
> and budget. I do not have dietary restrictions in the context available to this turn, so I
> will not infer them. I also cannot place restaurant bookings yet; authenticated web-driven
> booking is reserved for a future release. Shall I research and prepare a ready-to-book
> shortlist instead?

```text
CARD: INPUT · request input-uc2-gaps · answer scope and logistics once
 Prompt: Party size, dietary restrictions, budget, and agreement to research-only scope?
 Answer mode: one composite free-text answer · allowFreeText: true · multiSelect: false
 Step status: waiting
```

*(wire: `nyxid.input.request` · requestId=input-uc2-gaps · turnId=turn-uc2-1 · taskId=task-uc2 · stepId=step-uc2-gaps; revision 1 contains only this input step, with no tool read.)*

**User**
> Party of 6, one vegetarian, no budget cap - yes, do the shortlist. I understand no booking
> will be made.

*(wire: `input.resolve` · requestId=input-uc2-gaps · one freeText answer closes every gap and explicitly accepts the reduced scope.)*

**Assistant**
> Here's how I'll approach the agreed research-only task: the mounted Aevatar web-search skill
> will search Greek dinner candidates in northern Singapore for Friday between 6 and 7 pm,
> then I will compare fit for six people and one vegetarian. No reservation will be made by
> this task. The gate is derived as `auto` because every step only reads or drafts; nothing
> books, spends, or publishes. The plan is already running, and using existing read capability
> never costs a fresh permission click.

```text
CARD: PLAN · task task-uc2 · revision 2 · gate: auto - "read and draft only"
 1 ● done      You - answer logistics and agree to research-only scope · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ◐ running   Aevatar web search - find Greek dinner candidates · source: tool: aevatar-web-search-skill · addedBy: replan · status: running · externalEffect: not_started · availableActions: [stop]
   · running   Build one north-Singapore dinner query
   · running   Parse the single search result set
   · running   Filter the result set by area and Friday hours
 3 ○ planned   Assistant - compare shortlist fit · source: llm · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
```

*(wire: planRevision advances 1 -> 2 after the gap answer. `aevatar-web-search-skill` is source.kind=tool, not the reserved source.kind=web browser-driving executor.)*

*[narrator: the single search operation reports progress; its indented phases do not make
additional external calls.]*

```text
CARD: PLAN · task task-uc2 · revision 2 · gate: auto - "read and draft only"
 1 ● done      You - answer logistics and agree to research-only scope · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ◐ running   Aevatar web search - find Greek dinner candidates · source: tool: aevatar-web-search-skill · addedBy: replan · status: running · externalEffect: not_started · availableActions: [stop]
   · done      Build one north-Singapore dinner query
   · done      Parse the single search result set
   · running   Filter the result set by area and Friday hours
 3 ○ planned   Assistant - compare shortlist fit · source: llm · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
```

**User**
> Actually 7 pm sharp, and we need a private room.

*(wire: plain `text` would be rejected with ACTIVE_TURN_REQUIRES_STEERING; the front end sends `task.steer` with steeringId=steer-uc2-1 and expectedStateVersion, and a fence commits before successor work.)*

**Assistant**
> I have revised the plan for 7 pm sharp and a private room. The completed search and its
> results are preserved; I will not run it again. I am refining those results against the new
> constraints now.

```text
CARD: PLAN · task task-uc2 · revision 3 · gate: auto - "read and draft only"
 1 ● done      You - answer logistics and agree to research-only scope · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      Aevatar web search - find Greek dinner candidates · source: tool: aevatar-web-search-skill · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ⊗ cancelled Assistant - compare the original shortlist fit · source: llm · addedBy: replan · status: cancelled · externalEffect: not_started · availableActions: []
 4 ◐ running   Assistant - refine for 7 pm and a private room · source: llm · addedBy: steering · status: running · externalEffect: not_started · availableActions: [stop]
```

*(wire: task-uc2 continues on NEW turn turn-uc2-2 after steering; planRevision=3, completed evidence is preserved, and the replacement step has addedBy=steering.)*

**User**
> Hold off - we might do lunch instead. Stop.

*(wire: `task.stop` · turnId=turn-uc2-2 · stopRequestId=stop-uc2-1 · clientRequestId=client-stop-uc2-1 · expectedStateVersion; the stop fence commits before any successor decision.)*

```text
CARD: PLAN · task task-uc2 · revision 3 · gate: auto - "read and draft only"
 1 ● done      You - answer logistics and agree to research-only scope · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      Aevatar web search - find Greek dinner candidates · source: tool: aevatar-web-search-skill · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ⊗ cancelled Assistant - compare the original shortlist fit · source: llm · addedBy: replan · status: cancelled · externalEffect: not_started · availableActions: []
 4 ⊗ cancelled Assistant - refine for 7 pm and a private room · source: llm · addedBy: steering · status: cancelled · externalEffect: not_applied · availableActions: []
```

**Assistant**
> Stopped - this is a partial-work receipt, not a completed shortlist. The search found three
> possible candidates, but the private-room and 7 pm checks were cancelled. Nothing booked,
> published, or otherwise changed outside NyxID. Late evidence may refine what the search read,
> but it cannot advance this stopped plan.

*(wire: best-effort cancellation completed; task-uc2 and turn-uc2-2 terminalize with status=stopped, never succeeded.)*

*[narrator: twenty minutes later.]*

**User**
> Dinner's back on - finish the shortlist.

**Assistant**
> I will start a new research task using the prior results only as conversational context. The
> stopped task will not resume, and its steps will not be silently grafted here. The Aevatar
> search skill will check the three candidates for the private-room and 7 pm requirements in
> one fresh search operation, then I will prepare the shortlist. The gate is derived as `auto`
> because this remains pure read-and-draft work.

```text
CARD: PLAN · task task-uc2b · revision 1 · gate: auto - "new research task; no booking"
 1 ◐ running   Aevatar web search - check shortlisted candidates · source: tool: aevatar-web-search-skill · addedBy: initial · status: running · externalEffect: not_started · availableActions: [stop]
   · running   Check private-room evidence in one result set
   · running   Check Friday 7 pm evidence in the same result set
 2 ○ planned   Assistant - prepare the research shortlist · source: llm · addedBy: initial · status: planned · externalEffect: not_started · availableActions: [stop]
```

*(wire: `text` creates NEW turn turn-uc2b-1 and NEW task task-uc2b; task-uc2 remains stopped. The communicated approach precedes the new task's first tool call.)*

```text
CARD: PLAN · task task-uc2b · revision 1 · gate: auto - "new research task; no booking"
 1 ● done      Aevatar web search - check shortlisted candidates · source: tool: aevatar-web-search-skill · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      Assistant - prepare the research shortlist · source: llm · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
```

**Assistant**
> The research shortlist is ready. The search results do not state Atlas Taverna's Friday
> hours, and I did not open its site, so I cannot confirm them. I am not treating that as
> closed or unavailable.

```text
CARD: ARTIFACT · research shortlist - no reservation was made
 Recommended: North Olive, Yishun
   Private room: confirmed for 6 · Vegetarian fit: strong · Friday 7 pm: listed
   Booking: booking.example/north-olive · Phone: +65 6000 0124
 Second: Agora Table, Woodlands
   Private room: on request · Vegetarian fit: strong · Friday 7 pm: listed
   Booking: booking.example/agora-table · Phone: +65 6000 0187
 Cannot check right now: Atlas Taverna, Sembawang
   Private room: listed · Vegetarian fit: moderate · Friday hours: not stated in search results
   Booking: booking.example/atlas-taverna · Phone: +65 6000 0193
 Research artifact only: no reservation was made.
```

*(wire: task-uc2b is pure read-and-draft work, so section 7.7's verify-step requirement is exempt; no external effect is claimed.)*

**Assistant**
> Want me to draft the reservation request, or post this shortlist to the team on Lark?

## UC3 - Finance reimbursement from pasted invoices

*Precondition: `lark-feishu` is connected. Lark Approval is reachable through generated
operation tools.*

**User**
> File a reimbursement for these:
>
> 1. Cloud Market Pte Ltd - invoice INV-CM-8041 - 4 August 2026 - SGD 642.80.
> 2. DevTools Inc - invoice DT-19427 - 4 August 2026 - USD 219.00.
> 3. DevTools Inc - invoice DT-19427 - 4 August 2026 - USD 219.00.

*(wire: `text` creates `turn-uc3-1` and `task-uc3`; plan revision 1 begins.)*

**Assistant**
> Before I propose the filing approach, answer these together: expense category, cost center,
> and the reimbursement currency to use for the USD invoice. I have not extracted or deduped
> the invoices yet.

```text
CARD: INPUT · request input-uc3-gaps · answer finance fields once
 Prompt: Expense category, cost center, and reimbursement currency for the USD invoice?
 Answer mode: one composite free-text answer · allowFreeText: true · multiSelect: false
 Step status: waiting
```

*(wire: `nyxid.input.request` · requestId=input-uc3-gaps · turnId=turn-uc3-1 · taskId=task-uc3 · stepId=step-uc3-gaps; revision 1 contains only this input step, with no extraction or tool call.)*

**User**
> Category Software and cloud services, cost center ENG-PLATFORM, and reimburse the USD item
> in USD.

*(wire: `input.resolve` · requestId=input-uc3-gaps · one freeText answer closes every finance gap.)*

**Assistant**
> Here's how I'll approach it before doing any extraction: I will parse and dedupe the pasted
> text in one assistant operation, show you exactly what is kept or dropped, ask Lark Approval
> to create one reimbursement instance after NyxID's approval gate, then read that instance
> back to verify it exists. The gate is derived as `confirm` because filing creates an approval
> instance outside NyxID.

```text
CARD: PLAN · task task-uc3 · revision 2 · gate: confirm - "creates a Lark Approval instance"
 1 ● done      You - answer finance fields · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ⏸ waiting   You - approve this plan · source: input · addedBy: replan · status: waiting · externalEffect: not_started · availableActions: [stop]
 3 ○ planned   Assistant - extract and dedupe the pasted invoices · source: llm · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 4 ○ planned   Lark Approval - file the reimbursement · source: tool: lark-feishu__create_approval_instance · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 5 ○ planned   Verify - confirm the reimbursement exists · source: postcondition: lark-feishu__get_approval_instance · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
```

```text
CARD: INPUT · request input-uc3-gate · approve the communicated plan
 Option: Proceed (optionId: proceed)
 Free text: objections are folded into a re-plan
 Step status: waiting
```

*(wire: planRevision advances 1 -> 2; the execution steps are addedBy=replan, and the distinct gate request is `nyxid.input.request` requestId=input-uc3-gate.)*

**User**
> Proceed.

*(wire: `input.resolve` · requestId=input-uc3-gate · answer={selectedOptionIds:["proceed"]}.)*

```text
CARD: PLAN · task task-uc3 · revision 2 · gate: confirm - "creates a Lark Approval instance"
 1 ● done      You - answer finance fields · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      You - approve this plan · source: input · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ◐ running   Assistant - extract and dedupe the pasted invoices · source: llm · addedBy: replan · status: running · externalEffect: not_started · availableActions: [stop]
   · done      Parse the three pasted invoice sections
   · running   Compare invoice identities inside the same extraction operation
   · running   Normalize the two retained line items
 4 ○ planned   Lark Approval - file the reimbursement · source: tool: lark-feishu__create_approval_instance · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 5 ○ planned   Verify - confirm the reimbursement exists · source: postcondition: lark-feishu__get_approval_instance · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
```

**Assistant**
> Extraction complete. Invoice 3 is the same invoice as invoice 2: vendor DevTools Inc,
> number DT-19427, date, currency, and amount all match. I am dropping invoice 3 and filing two
> items, not three.
>
> | Kept item | Category | Cost center | Amount |
> |---|---|---|---:|
> | INV-CM-8041 - Cloud Market | Software and cloud services | ENG-PLATFORM | SGD 642.80 |
> | DT-19427 - DevTools | Software and cloud services | ENG-PLATFORM | USD 219.00 |

```text
CARD: NYXID APPROVAL · nyxid request nyxid-appr-uc3-1 · awaiting decision
 Badge: Decided on NyxID - not by the assistant
 Action: Create one reimbursement approval instance with 2 line items
 Target: Lark Approval · ENG-PLATFORM
 Actor: Calvin via NyxID Chat
 Reversibility: unknown
 grantBoundary: nyxid_step_up
 Countdown: 09:51 remaining
 Other decision surfaces: NyxID mobile, Telegram, dashboard
```

*[narrator: Calvin double-clicks Approve on the embedded NyxID card. NyxID commits the first
decision; the second click is an idempotent duplicate and creates neither a second decision
nor a second reimbursement instance.]*

*(wire: generation 1's pre-effect dispatch raised nyxidRequestId=nyxid-appr-uc3-1; the request is decided on NyxID, Aevatar observes the outcome through an honest wake/recheck, and the exact grant re-enters the tool at generation 2. G8's bridge remains unspecified.)*

<details>
<summary><strong>Alternate outcome - approval countdown expires</strong></summary>

```text
CARD: NYXID APPROVAL · nyxid request nyxid-appr-uc3-1 · expired
 Badge: Decided on NyxID - not by the assistant
 Outcome: Expired; treated as denial
```

```text
CARD: PLAN · task task-uc3 · revision 2 · gate: confirm - "creates a Lark Approval instance"
 1 ● done      You - answer finance fields · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      You - approve this plan · source: input · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ● done      Assistant - extract and dedupe the pasted invoices · source: llm · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 4 ✕ failed    Lark Approval - file the reimbursement · source: tool: lark-feishu__create_approval_instance · addedBy: replan · status: failed · outcome: denied (approval expired) · externalEffect: not_applied · availableActions: []
 5 ⊗ cancelled Verify - confirm the reimbursement exists · source: postcondition: lark-feishu__get_approval_instance · addedBy: replan · status: cancelled · externalEffect: not_started · availableActions: []
```

**Assistant**
> The NyxID approval expired, so it was treated as denied. Nothing was filed, the verification
> step will not run, and this required path has stopped. Ask me to file it again when you are
> ready; NyxID will create a fresh approval request.

</details>

*[narrator: the primary transcript continues from the approved outcome.]*

**Assistant**
> Lark returned a gateway error after the request was dispatched. The extraction completed,
> but the reimbursement may have been created. I cannot safely retry an effect-capable call
> until I reconcile Lark's state, so only Stop is available right now.

```text
CARD: PLAN · task task-uc3 · revision 2 · gate: confirm - "creates a Lark Approval instance"
 1 ● done      You - answer finance fields · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      You - approve this plan · source: input · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ● done      Assistant - extract and dedupe the pasted invoices · source: llm · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 4 ? uncertain Lark Approval - file the reimbursement · source: tool: lark-feishu__create_approval_instance · addedBy: replan · status: uncertain · externalEffect: may_have_changed · availableActions: [stop]
 5 ○ planned   Verify - confirm the reimbursement exists · source: postcondition: lark-feishu__get_approval_instance · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
```

*(wire: operation phase is uncertain after the Lark 502; uncertain is not success and not a retry invitation, and may_have_changed prevents replay.)*

**Assistant**
> I am adding a read-only reconciliation step: list today's Lark Approval instances for this
> submitter and check for the exact two-item reimbursement.

```text
CARD: PLAN · task task-uc3 · revision 3 · gate: confirm - "creates a Lark Approval instance"
 1 ● done      You - answer finance fields · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      You - approve this plan · source: input · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ● done      Assistant - extract and dedupe the pasted invoices · source: llm · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 4 ✕ failed    Lark Approval - file the reimbursement · source: tool: lark-feishu__create_approval_instance · addedBy: replan · status: failed · externalEffect: not_applied · availableActions: [retry, skip, stop]
 5 ● done      Lark Approval - reconcile today's instances · source: tool: lark-feishu__list_approval_instances · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 6 ○ planned   Verify - confirm the reimbursement exists · source: postcondition: lark-feishu__get_approval_instance · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
```

*(wire: `lark-feishu__list_approval_instances` found no exact submitter/day/items match, proving the failed generation was not_applied; only then did the actor compute availableActions=[retry,skip,stop].)*

*[narrator: Calvin clicks Retry on the failed filing step.]*

*(wire: `step.retry` · taskId=task-uc3 · stepId=step-uc3-file · retryRequestId=retry-uc3-1 · expectedOperationGeneration=2 · expectedStateVersion; generation 3 is current generation + 1. Its pre-effect proxy admission raises a fresh `nyxid.approval.request` before any downstream mutation executes.)*

```text
CARD: NYXID APPROVAL · nyxid request nyxid-appr-uc3-2 · awaiting decision
 Badge: Decided on NyxID - not by the assistant
 Action: Retry creation of one reimbursement approval instance with 2 line items
 Target: Lark Approval · ENG-PLATFORM
 Actor: Calvin via NyxID Chat
 Reversibility: unknown
 grantBoundary: nyxid_step_up
 Countdown: 09:58 remaining
 Other decision surfaces: NyxID mobile, Telegram, dashboard
```

*[narrator: Calvin approves the fresh retry request on the embedded NyxID card.]*

*(wire: pending approval nyxidRequestId=nyxid-appr-uc3-2 is decided on NyxID; Aevatar observes the outcome through an honest wake/recheck. The exact retry grant re-enters generation 3 at generation 4; the spent nyxid-appr-uc3-1 step-up is not reused.)*

```text
CARD: PLAN · task task-uc3 · revision 3 · gate: confirm - "creates a Lark Approval instance"
 1 ● done      You - answer finance fields · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      You - approve this plan · source: input · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ● done      Assistant - extract and dedupe the pasted invoices · source: llm · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 4 ◐ running   Lark Approval - file the reimbursement · source: tool: lark-feishu__create_approval_instance · addedBy: replan · status: running · externalEffect: not_started · availableActions: [stop]
 5 ● done      Lark Approval - reconcile today's instances · source: tool: lark-feishu__list_approval_instances · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 6 ○ planned   Verify - confirm the reimbursement exists · source: postcondition: lark-feishu__get_approval_instance · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
```

```text
CARD: PLAN · task task-uc3 · revision 3 · gate: confirm - "creates a Lark Approval instance"
 1 ● done      You - answer finance fields · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      You - approve this plan · source: input · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ● done      Assistant - extract and dedupe the pasted invoices · source: llm · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 4 ● done      Lark Approval - file the reimbursement · source: tool: lark-feishu__create_approval_instance · addedBy: replan · status: done · externalEffect: confirmed · availableActions: []
 5 ● done      Lark Approval - reconcile today's instances · source: tool: lark-feishu__list_approval_instances · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 6 ● done      Verify - confirm the reimbursement exists · source: postcondition: lark-feishu__get_approval_instance · addedBy: replan · status: done · externalEffect: confirmed · availableActions: []
```

*(wire: `lark-feishu__get_approval_instance` read the new instance back and matched both retained items before RUN_FINISHED(completed).)*

**Assistant**
> The reimbursement is filed and verified.

```text
CARD: ARTIFACT · reimbursement instance REIM-2026-0805-41
 Status: Pending manager approval
 Approver: Priya Raman
 Cost center: ENG-PLATFORM
 Items: 2 retained · 1 duplicate explicitly dropped
 Amounts: SGD 642.80 + USD 219.00
 Link: lark.example/approval/REIM-2026-0805-41
 Verified against Lark Approval: the instance exists with both retained line items and is pending.
```

## UC4 - HR resume screen and conditional tracker write

*Precondition: `lark-feishu` is connected and a Candidate Tracker Bitable exists.*

**User**
> Screen this for the Senior Backend Engineer role - if they clear our bar, add them to the
> candidate tracker.
>
> Maya Chen. Seven years building backend platforms. Recent work includes Rust services,
> MongoDB data systems, distributed job orchestration, OAuth integrations, and incident lead
> rotations. Led a four-person migration from synchronous workers to durable queues.

*(wire: `text` creates `turn-uc4-1` and `task-uc4`; plan revision 1 begins.)*

**Assistant**
> I need one combined answer before I can propose a fair screen: paste the job description or
> scoring criteria, set the passing bar (I suggest 70/100), name the tracker table, and give
> the stage tag. I will not invent a hiring rubric.

```text
CARD: INPUT · request input-uc4-gaps · supply the rubric and write conditions once
 Prompt: JD criteria, passing bar, tracker table, and stage tag?
 Answer mode: one composite free-text answer · allowFreeText: true · multiSelect: false
 Step status: waiting
```

*(wire: `nyxid.input.request` · requestId=input-uc4-gaps · turnId=turn-uc4-1 · taskId=task-uc4 · stepId=step-uc4-gaps; revision 1 contains only this input step, with no inventory read or scoring pass.)*

**User**
> Score five criteria equally: Rust production services, MongoDB/data modeling, distributed
> systems, OAuth/OIDC, and incident leadership. Passing bar 75. Use the "2026 Pipeline" table
> and stage "screen".

*(wire: `input.resolve` · requestId=input-uc4-gaps · one freeText answer supplies the complete user-authored rubric and write conditions.)*

**Assistant**
> I checked the relevant capability: Lark is connected and executable. Here's how I'll
> approach it: I will score the resume once against the five criteria you supplied, show the
> rationale, and only if the score is at least 75 will Lark write one row to the table you
> named after NyxID's approval gate. If the score is below 75, the write step will show the
> literal status `skipped`; it will not silently vanish. A Lark read will verify any written
> row. The gate is derived as `confirm` because the conditional plan contains an external
> write.

```text
CARD: PLAN · task task-uc4 · revision 2 · gate: confirm - "conditionally writes a Bitable row"
 1 ● done      You - provide the JD rubric and write conditions · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ⏸ waiting   You - approve this plan · source: input · addedBy: replan · status: waiting · externalEffect: not_started · availableActions: [stop]
 3 ○ planned   Assistant - score against Calvin's five JD criteria · source: llm · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 4 ○ planned   Lark Candidate Tracker - add the candidate if score >= 75 · source: tool: lark-feishu__create_bitable_record · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 5 ○ planned   Verify - confirm the candidate row exists · source: postcondition: lark-feishu__get_bitable_record · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
```

```text
CARD: INPUT · request input-uc4-gate · approve the communicated plan
 Option: Proceed (optionId: proceed)
 Free text: objections are folded into a re-plan
 Step status: waiting
```

*(wire: disclosed Class-R readiness precedes `nyxid.task.snapshot`; planRevision advances 1 -> 2, and input-uc4-gate is distinct from input-uc4-gaps.)*

**User**
> Proceed.

*(wire: `input.resolve` · requestId=input-uc4-gate · answer={selectedOptionIds:["proceed"]}.)*

```text
CARD: PLAN · task task-uc4 · revision 2 · gate: confirm - "conditionally writes a Bitable row"
 1 ● done      You - provide the JD rubric and write conditions · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      You - approve this plan · source: input · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ◐ running   Assistant - score against Calvin's five JD criteria · source: llm · addedBy: replan · status: running · externalEffect: not_started · availableActions: [stop]
   · done      Parse resume evidence in one scoring operation
   · running   Score against the five supplied criteria
   · running   Prepare evidence-linked rationale
 4 ○ planned   Lark Candidate Tracker - add the candidate if score >= 75 · source: tool: lark-feishu__create_bitable_record · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
 5 ○ planned   Verify - confirm the candidate row exists · source: postcondition: lark-feishu__get_bitable_record · addedBy: replan · status: planned · externalEffect: not_started · availableActions: [stop]
```

**Assistant**
> Maya Chen scores **82/100 against the JD criteria you provided**, clearing the bar of 75.
> The strongest evidence is production Rust work and distributed queue migration. MongoDB and
> OAuth experience are directly stated. Incident leadership is credible but described with
> less scope detail than the engineering work.
>
> Rust 19/20 · MongoDB/data modeling 15/20 · distributed systems 17/20 · OAuth/OIDC 16/20 ·
> incident leadership 15/20.

```text
CARD: NYXID APPROVAL · nyxid request nyxid-appr-uc4-1 · awaiting decision
 Badge: Decided on NyxID - not by the assistant
 Action: Add Maya Chen with score 82 and stage screen
 Target: Lark Candidate Tracker · 2026 Pipeline
 Actor: Calvin via NyxID Chat
 Reversibility: reversible
 grantBoundary: nyxid_step_up
 Countdown: 08:14 remaining
 Other decision surfaces: NyxID mobile, Telegram, dashboard
```

*[narrator: Calvin reloads the page before deciding.]*

*(wire: `GET /api/chat/conversations/conversation-calvin/state` returns the same TaskPlan shape and pending approval facts; the UI rehydrates from state, not frame replay.)*

```text
CARD: NYXID APPROVAL · nyxid request nyxid-appr-uc4-1 · awaiting decision
 Badge: Decided on NyxID - not by the assistant
 Action: Add Maya Chen with score 82 and stage screen
 Target: Lark Candidate Tracker · 2026 Pipeline
 Actor: Calvin via NyxID Chat
 Reversibility: reversible
 grantBoundary: nyxid_step_up
 Countdown: live after reload
 Other decision surfaces: NyxID mobile, Telegram, dashboard
 State: same pending decision; nothing duplicated
```

*[narrator: Calvin approves from the NyxID mobile app instead of the chat card. The mobile
decision is recorded first.]*

```text
CARD: NYXID APPROVAL · nyxid request nyxid-appr-uc4-1 · decided
 Badge: Decided on NyxID - not by the assistant
 Outcome: Approved on mobile
 Chat controls: display-only
 First decision wins; every other linked surface now only displays the outcome.
```

*(wire: Aevatar observes the NyxID outcome through an honest wake/recheck and the gated write proceeds; G8's internal bridge is not named or invented.)*

```text
CARD: PLAN · task task-uc4 · revision 2 · gate: confirm - "conditionally writes a Bitable row"
 1 ● done      You - provide the JD rubric and write conditions · source: input · addedBy: initial · status: done · externalEffect: not_applied · availableActions: []
 2 ● done      You - approve this plan · source: input · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 3 ● done      Assistant - score against Calvin's five JD criteria · source: llm · addedBy: replan · status: done · externalEffect: not_applied · availableActions: []
 4 ● done      Lark Candidate Tracker - add the candidate if score >= 75 · source: tool: lark-feishu__create_bitable_record · addedBy: replan · status: done · externalEffect: confirmed · availableActions: []
 5 ● done      Verify - confirm the candidate row exists · source: postcondition: lark-feishu__get_bitable_record · addedBy: replan · status: done · externalEffect: confirmed · availableActions: []
```

*(wire: `lark-feishu__get_bitable_record` matched the candidate, score, stage, and table before RUN_FINISHED(completed).)*

**Assistant**
> The candidate row is written and verified.

```text
CARD: ARTIFACT · Candidate Tracker entry - Maya Chen
 Score: 82/100, scored against the JD Calvin provided
 Breakdown: Rust 19 · MongoDB/data modeling 15 · distributed systems 17 · OAuth/OIDC 16 · incident leadership 15
 Bar: 75 · Result: clears bar
 Table: 2026 Pipeline · Stage: screen
 Link: lark.example/bitable/2026-pipeline/maya-chen
 Verified against Lark Candidate Tracker: the row exists with the written score and stage.
```

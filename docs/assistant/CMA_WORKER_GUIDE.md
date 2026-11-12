# CMA Worker Guide

Contract version: 1.

CMA requests use the selected NyxID service connection and its effective endpoint.
Creation, confirmation, management, events and Desktop stay on that original
connection. Keep its service identity with the original body and Idempotency-Key.
If the connection is removed, disabled, loses access or points somewhere else,
stop recovery and report the change; do not switch to a default connection.
Historical creations without a saved original connection cannot be recovered
through this management flow. Do not guess a connection or rotate their keys.
This guide explains operation; current authorization and backend admission remain authoritative.

Use CMA when it helps the task. Decide whether to reuse an existing CMA Agent,
create a new Agent, or use other tools. Each account may have at most one
non-retired Sandbox created through this NyxID instance. That Sandbox may contain
several Agents. External CMA Sandboxes do not use this allocation and are not
imported into NyxID management. CMA's own quotas still apply independently.
Use nyxid__list_cma_resources to inspect the NyxID-created environment and its Agents.
Match a user-named Agent to its returned label and agent_id. Different Agents in
one Sandbox have independent state; never transfer a sibling's status or approval.
last_known_response_id is a previously observed Response for that Agent, including
work started in another NyxBot chat. It is a lookup hint, not proof of a current
task. Use the current CMA OpenAPI discovery result to select the GET operation for
`/api/v2/responses/{response_id}` and the GET controls operation for
`/api/v2/agents/{agent_id}/responses/{response_id}/controls`; identify operations
by HTTP method and path, not by a fixed tool name. Verify its conversation.id,
target_active, and current revision before proposing any control. Never invent a
response_id or treat an old/completed Response as the current task. If this hint
is absent or no longer active, report that the current task cannot yet be identified.
Create additional Agents inside that environment with sandbox_id or workspace_id.
Create a Sandbox only when this allocation is empty, no creation is unresolved,
and the task needs it. A list is not a reservation.
Before preparing each NEW Agent or Sandbox creation, call nyxid__list_cma_resources
and copy its new_creation_idempotency_key into the creation's Idempotency-Key.
Use that key for only this creation intent. Do not invent date/counter keys or
reuse a key from another creation or chat. Keep the chosen key and exact body
through action confirmation, credential setup, and receipt-loss recovery.
A later inventory read returns a key for a different future creation; it must
never rotate the key of a pending or uncertain request. Do not automatically
retry a conflicting creation with a new key to bypass the original identity.
For the discovered `POST /api/v1/agents` operation, the inline
`profile.permission_mode` accepts guarded or autonomous.
When the user wants approval before commands or file changes, explicitly select
guarded. Do not invent ask/default permission values or silently omit the requested mode.
For an ordinary request for a shared desktop (including "带共享桌面" or
"Include a shared desktop"), set machine_included=true when creating a new
Sandbox. This option does not require desktop=true or initial_message. If the
user only wants the Agent created, omit both fields and wait for later work;
do not ask for or invent a first task just to include a shared desktop. When
reusing an existing parent, omit machine_included and check its desktop capability.
Only use desktop=true when the user specifically requests CMA's separate desktop
preparation-and-first-task flow. That flow requires the user's exact first task
in initial_message and verified runtime support. Its unsupported result does
not establish that the machine_included shared-desktop option is unavailable.
Preserve the requested options and the exact body/key when recovering a creation.
Run work with the currently discovered CMA operation for
`POST /api/v2/agents/{agent_id}/responses`, using the selected agent_id,
`stream=true`, and a stable Idempotency-Key. If conversation is supplied, it must
equal that agent_id. Keep the request fields and command enum from the discovered
schema unchanged; operation names may vary between CMA document revisions.
After nyxid__cma_save_attachment, use the returned absolute_path in Agent commands.
Its path field is relative to the CMA filesystem API root /home/; the Agent's
working directory may be a separate worktree.
Reusing an Agent shares its conversation and worktree, including across NyxBot chats.
Keep the exact body and Idempotency-Key when recovering an uncertain request.
Manage Agent lifecycle through tools. The user can inspect an Agent's events, history,
and available shared desktop on its execution page. The Sandboxes page manages the
parent environment and does not provide a desktop viewer.
Distinguish task cancellation, Agent stop/delete, and whole-Sandbox retirement.
Deleting an Agent does not retire its Sandbox or guarantee that resource charges stop.
Report observed queueing, failures and cleanup outcomes without claiming unobserved success.
NyxID action confirmation and CMA native approval are separate. Denying a NyxID
card sends nothing to CMA; it does not reject a pending cloud approval or cancel
the cloud task. Explain that the cloud request may still be waiting. If the user
explicitly asks to refuse a cloud approval, send resolve_approval with decision=deny
for the exact observed pending approval, subject to the normal NyxID confirmation.
If they ask to stop the task, use the exact task cancellation instead. Never change
an already confirmed control body to its opposite or retry a denied action.
A queued control or allowed confirmation is not completed work. Check the control
outcome and the exact Response or transcript before claiming that a file was written,
an approval was resolved, or a task stopped. incomplete with interrupted denotes a
cancelled CMA response; ordinary incomplete is not proof of cancellation. A NyxID
tracking timeout only ends tracking: query the live task before describing its outcome.
A completed response that asks a question in plain text is not a native pending
approval. When its Agent is ready and the user answers, forward the answer with
the currently discovered `POST /api/v2/agents/{agent_id}/responses` operation on
that same Agent. Use resolve_approval only
after observing the exact native pending approval and its current response/revision;
never invent one from wording or a failed read. A failed read means unknown, not
waiting_approval.

Pause and revive the exact observed Sandbox using its current generation and a
stable request key. Pause preserves the allocation. Retirement releases the
NyxID slot only after confirmed retirement; a queued command or failed read is
not proof that it is free. Preserve uncertain creation identity and occupancy.

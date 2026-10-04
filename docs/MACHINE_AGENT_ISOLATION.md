# M1 draft: agent-scoped machine workspaces, authority, and visibility

Status: design approved on 2026-10-03; revised M1.3 backend/runtime slice ready for review. Owner rollout is incomplete; see §8.7. Started incrementally at `/tmp/m1-design.md`, moved here with owner authorization after #1747 merged.
Date: 2026-10-03.
Initial audit baseline: NyxID `origin/main` = `52aa526550db3ef84946b6032795f5ec36c20166` (v0.47.0, #1746). After #1747 merged, fetched and branched from `36e3ef04b571f80bafab0fee88cc2b77ac5abaf8` (v0.48.0). Reviewed the intervening diff: the machine isolation findings are unchanged; the UI design now explicitly uses U1’s overlay primitives. Source line references below refer to the initial audit unless a symbol/link identifies the final baseline. OpenDots downloaded reference plus GitHub client source fetched at HEAD `c2569bb6a13a22e565cf3eb791c62267d06babb1` (2026-10-02). The supplied five reference files and both fetched client files were verified byte-for-byte against GitHub blob hashes at that OpenDots revision. `computer-store.ts` was also fetched pinned to that revision. Initial research used `git show origin/main:<path>` without branch or repository mutation. The authorized design branch is now `feat/machine-agent-isolation`; M1.1 proceeds on this branch without new machine authority.

## 1. Executive recommendation

Adopt agent-scoped machine grants and execution contexts, plus contextual chat cards and a metadata-only activity view. Do not call a directory prefix or a Chromium profile an isolation boundary: shell commands need enforced OS separation, and browser contexts need separate displays, sockets and buses. Keep today's shared behavior explicitly labeled for existing installations; make new grants deny-by-default and offer separated contexts only on nodes that can enforce them. Defer a Docker-socket supervisor to a later, optional feature; separate existing machine containers already provide a stronger boundary today.

Recommended priority: make the current sharing visible; add explicit capability grants and immediate revocation; deliver enforced Linux execution contexts; then add per-context browsers and saved-login boundaries. Safe cards/activity can ship first and do not require container orchestration.

## 2. Evidence and current behavior

### 2.1 OpenDots computer model

The supplied `docs/COMPUTERS.md` and `deployment/computers/README.md` describe one persistent computer/container per Dot, workspace/profile volumes surviving stop/start, and a supervisor pinned to OpenBot `b6932d31a8d6e7896c15139dfc27a6c6911deb27`. The master computer token stays in the app/supervisor; child credentials are HMAC-SHA256 scoped to Dot ID (`computer-service.ts:41`). The endpoint resolver verifies the returned bot/container and permits only the corresponding container DNS or verified loopback endpoint (`:139`). Browser/files/shell capabilities are per Dot and default disabled by the documented contract. Request execution rechecks permissions, polls every 50 ms and aborts its HTTP request on revocation (`:311`). Documentation honestly says cancellation may not stop an already-started upstream effect.

The comparison is not evidence of a stronger browser secret boundary: OpenDots explicitly says shell can read that Dot's browser profile, uses the shared host kernel, and has no restrictive egress policy. gVisor is an optional already-installed runtime. NyxID must preserve its stronger secure-browser/agent-user separation rather than copy this limitation.

### 2.2 NyxID: workspace, browser, and saved-login sharing

**Yes: two specialists granted the same machine share its workspaces and browser sessions today.** `AssistantAgent.machine_node_ids` and `saved_login_ids` are flat lists (`backend/src/models/assistant_agent.rs:142`). `machine_service::granted` treats membership as access to the entire node, with NyxBot implicitly allowed (`backend/src/services/machine_service.rs:130`). Node `Runtime` owns one roots set, one command identity, one secure browser, one dev browser and one pair of displays (`cli/src/node/machine/runtime.rs:149–230`), independent of the agent ID.

- File tools are confined to configured roots with canonicalization and descriptor-relative `O_NOFOLLOW`; the root set is node-wide (`files.rs:129–185`). There is no per-specialist subtree. Both specialists can read/change files under the same permitted roots.
- Shell commands use the same configured `agent_user`, `HOME` and bounded cwd; cwd validation is not a filesystem sandbox (`jobs.rs:85–125`, `process.rs:137–182`). A mere per-agent cwd would not stop `../`, absolute paths, same-UID file/process access, shared caches, or sockets.
- Secure browser access reaches the one runtime browser, so a specialist may operate another specialist's authenticated tabs even without that saved login grant. The secure profile/cookie files themselves are protected from the command UID in separated installations; that is agent-versus-browser isolation, not specialist-versus-specialist isolation.
- Saved-login materialization already checks each specialist's `saved_login_ids`, origins, secure-browser-only use, confirmation and isolated-browser policy (`handlers/machine_tools.rs:167–207`). This restricts *new credential filling*, not use of cookies already present in the shared browser. Existing login grants should be retained and contextualized, not replaced or described as absent.
- The job API is stronger: retrieval binds node, conversation, API key and person; `MachineJob` also records agent ID (`machine_service.rs:190–252`). This prevents cross-thread job polling through the API, but not shared-UID/shared-workspace interference.

### 2.3 NyxID: capability grants and revocation

Local config has three booleans (`shell`, `files`, `computer`), default false (`machine/src/config.rs:8–51`). Browser is currently a supported tool protocol under the `computer` capability, not a fourth independent permission: `Operation::Browser`, `Computer` and `FillLogin` all require `profile.computer` (`machine/src/lib.rs:115–140`). Server checks the node profile and machine membership, not a per-(agent,machine) capability set (`machine_service.rs:130–147`, `handlers/machine_tools.rs:101–107`).

Machine calls already require a live non-stopped turn and add the node to that turn before dispatch (`handlers/machine_tools.rs:290–306`). The node subscribes to per-conversation/turn cancellation and owner-control epochs, uses biased cancellation selection and refuses late results (`runtime.rs:539–593`). These are existing boundaries to extend, not reinvent. Grant removal currently updates the agent and thread authorities transactionally and expires permission requests, but does not call the signed node cancellation path (`assistant_team_service.rs:1183–1345`). It therefore refuses subsequent calls without necessarily ending an already-running command, file action or browser request. `machine_gateway::job_auth` reloads the key, actor/org authority, node/runtime and machine grant on each new gateway request (`handlers/machine_gateway.rs:554–619`). That is not an in-flight stream revocation guarantee. Preserve this fresh authorization and the existing B1 operation checks on final proxy routes; add a durable revocation signal to running work, not a second service credential path.

Owner takeover is currently display-specific for secure/dev browser calls on Linux, and machine-wide for shell/files or the shared macOS desktop (`handlers/machine_tools.rs:137–164`, `runtime.rs:359–403`). It is not an agent-to-agent browser scheduling lock. A browser mutex serializes individual operations, not a multi-call task's focus/tab assumptions.

### 2.4 NyxID: chat cards and machine activity

- `TurnActivity` is metadata-only: tool label, status, start/end; the latest 40 are retained (`models/assistant_conversation.rs:81`, `services/assistant_nyxagent.rs:1969`). MCP wraps dispatch with activity recording (`handlers/mcp_transport.rs:1533–1626`), but completion currently uses HTTP success, which can mislabel a JSON/MCP error transported with HTTP 200 as completed. A machine card must use the typed operation/job outcome.
- `chat-message.tsx:123` renders generic tool details (up to 500 characters when a result exists). Live machine desktops are rendered separately in `assistant-chat-page.tsx:708`; there is no first-class per-machine operation card linked to the corresponding action.
- The existing desktop panel already has live streaming, secure/dev switching, cursor/action overlay, collapse, pop-out/fullscreen, Stop and takeover. Reuse it; do not introduce polling a screenshot per card.
- Image attachments are envelope-encrypted, thread/owner scoped and retention-checked before and after decryption (`assistant_nyxagent.rs:1912–1966`). `ToolImage` loads authenticated blobs, revokes object URLs and renders expired placeholders. This is the correct thumbnail substrate.
- `machine_operation` audit entries include node, conversation, role, API key attribution, outcome, duration and selected services, but not stable agent ID, action subtype or a correlation ID for the specific activity/job (`handlers/machine_tools.rs:393–480`). Job rows already have agent ID but cover shell only and do not retain command/output.
- `MachinesPage` currently lists machines, settings, saved logins and Watch/Take over; there is no per-agent activity view (`frontend/src/pages/machines.tsx`). The grant picker is a list of machine/login checkboxes, not a capability matrix (`machine-grant-picker.tsx`).

OpenDots makes these concepts easier to find: Browser / Files / Terminal / Activity tabs (`ComputerPanel.tsx:14,217,587`) and action-specific inline cards with state, text excerpts and an expandable browser preview (`ComputerToolCard.tsx:46–203`). Its card shows the *current* screen, polling every three seconds, not an immutable screenshot of the historical tool call. NyxID should label that distinction explicitly and reuse its existing stream instead of copying the polling implementation.

### 2.5 Existing NyxID guarantees to preserve

| Area | NyxID behavior that must survive M1 |
|---|---|
| Credential safety | Server-side encrypted saved logins; exact origin/field enforcement; protected input refusal; secure browser without arbitrary evaluation or DevTools; dev browser never receives saved logins. |
| OS/browser safety | Dedicated command, secure-browser and dev-browser UIDs in separated Linux installs; distinct secure/dev Xauthority; Chromium renderer sandbox; namespace-denying agent seccomp; no Docker socket in the machine. |
| Network authority | Per-job gateway credentials instead of provider keys in job environments; current actor/org/platform ACL, B1 operation scopes, approval and billing checks stay on the final route. Arbitrary internet access is not claimed to be eliminated. |
| Stop/takeover | Signed replay-protected commands, runtime/turn fencing, process-group cancellation, gateway aborts, late-result refusal; owner input has an independent path. |
| Browser fidelity | Trusted input + hit testing, visible frames, scoped/query/paginated snapshots, AX for fallback, automatic driver/extension recovery, persistent-profile repair. |
| Owner privacy | Memory-only live desktop frames, encrypted conversation attachments, retention/purge, no typed values/full commands in metadata audit. |
| Lifecycle | Attested pinned images, private updater mailbox/TUF store, journaled machine and companion rollback, guided upgrades and durable watches. |

OpenDots is ahead in per-agent lifecycle/UX and continuously rechecked capability revocation. It is not evidence that NyxID should discard its login separation, signed node protocol, stronger stop semantics, attachment privacy, or update guarantees. Neither design is a VM boundary or a general protection against a kernel exploit.

## 3. Proposed behavior and security boundaries

### 3.1 Separate workspace and browser for this agent

The owner-approved mode is `separated`; the UI label is **Separate workspace and
browser for this agent**. It combines the former workspace and browser phases.
`shared_legacy` remains the default. **Full isolation requires a separate machine
container or VM per agent.** Never label this mode “isolated”.

#### What separated does and does not guarantee

Each server-derived agent/person/group context receives command, secure-browser
and developer-browser OS users whose UIDs are never reused. Workspace, home, tmp,
browser profiles and native sockets have private DAC boundaries. Supervisor-owned
ancestors prevent a context from widening access to siblings by chmod or rename.
Legacy roots become supervisor-owned with only the legacy group able to traverse
them; world-writable files below them remain unreachable to context UIDs. This
change happens only as part of the owner's explicit opt-in, not on upgrade.

Linux Landlock ABI **6 or later**, verified by a real execution probe, provides
additional read/write/execute restrictions and signal/abstract-socket scoping.
Command and file-worker execution keeps no-new-privileges, the existing namespace-denying agent
seccomp filter, a cleared environment, private temporary directory and descriptor
cleanup using CLOEXEC/close_range. Its runtime allowlist is narrow and read-only.
Browser processes keep Chromium’s own sandbox, NNP, private DAC gates, descriptor
cleanup and Landlock signal/abstract-socket scopes. They do not receive the command
filesystem ruleset: Chromium must write child `/proc` namespace mappings to create
its own sandbox, which a read-only `/proc` grant prevents. We neither disable that
sandbox nor pretend a broad writable `/proc` grant is a narrow runtime allowlist.
These mechanisms are cumulative: UID separation alone is not this mode.

`/dev/shm` is intentionally available for POSIX semaphores and Python process
pools. Distinct UIDs, mode 0600 and the sticky directory protect other contexts'
segments. Names are visible and capacity is shared, so exhaustion is possible.
Landlock does not mediate all metadata operations: world-writable system files
outside private DAC ancestors may permit timestamp changes, and files owned by
the context UID outside its allowlist may permit chmod. Public pathname Unix
sockets remain reachable; private sockets require protected ancestors. Network,
PID and IPC namespaces, kernel, supervisor and resource capacity remain shared.
Browser runtimes need `/proc` for Chromium and accessibility; DAC/ptrace checks
protect other UIDs’ environment, memory and descriptors, but process metadata
(including command lines) can be visible. Keep secrets out of command-line arguments.
This is not hostile multi-tenant isolation or protection from kernel exploits,
shared-host DoS, public local services, or shared remote-account state.

Each context has secure and dev browser UIDs, separate profiles, Xvfb displays,
Xauthority cookies and D-Bus sessions. DevTools is dev-only. Command children
receive no display cookie, native socket or browser credentials. Saved-login fills
bind to the signed context and secure-profile generation; revocation closes and
quarantines that generation before a fresh profile can be used. Profiles and
cookies are never copied from the legacy browser or between contexts. Private X
servers disable their abstract listener and retain the cookie-authenticated
pathname socket, so Landlock scoping never requires a shared display exception.
D-Bus and accessibility activation receive the context's own display and private
runtime directory; their children also inherit scoped execution. The private
AT-SPI service is activated and enabled before Chromium starts, including for a
fresh OS user. Container startup preserves the legacy workspace's supervisor-owned
DAC gate once a context allocation journal exists.

On native Linux, new developer UIDs receive named-user POSIX ACL denials on secure
policy/native-host files and read access to a private developer policy. Existing
profiles retain their policy access. ACL setup errors refuse the context rather
than weakening another profile. The durable allocation journal retains UID and
profile-generation tombstones; the current bounds are 128 contexts and 128
browser generations per context. Exhaustion refuses provisioning, never reuses a
UID. Preserve this journal with the machine identity/workspace volume.

macOS, single-user Linux and Linux without a successful ABI 6 probe cannot enable
`separated`. Return an explicit unavailable reason; never fall back to shared
execution. Protocol v2 alone is insufficient: the node must separately advertise
verified separated support. The default-off `assistant:machine-contexts` flag gates
new opt-ins, and an owner action card names the fresh workspace/browser and the
interruption of existing work. Stored enforcement remains active with the flag off.

The original spike's failed stronger boundary remains documented in
[MACHINE_CONTEXT_SPIKE.md](MACHINE_CONTEXT_SPIKE.md). Revised adversarial tests must
retain the metadata, public pathname-socket and shared-memory residuals as explicit
**allowed-by-design controls**, alongside cross-context denials and tooling tests.

### 3.2 Capability grants, saved logins, and NyxBot policy

Store explicit per-(agent,machine) grants beside `machine_node_ids`, not inside service `grants`. With the capability editor enabled for the acting person, each new assignment starts with all four tool capabilities off. While that flag is off, the existing pairing and Grants flows snapshot legacy capabilities under the live ACL (§8.4). Effective authority is the intersection of: live actor/org access, machine membership, explicit agent capability, node-local capability, supported protocol/isolation mode, live grant revision, live turn/job, owner-control fence, and applicable owner confirmation. Saved-login filling adds its own live login grant and origin/field gates; service gateway use adds B1 and current service grants/ACLs/billing. Discovery and execution use the same projection; ungranted capabilities have an actionable permission-request flow rather than an opaque missing tool.

| Grant | Native operations | Additional rule |
|---|---|---|
| `shell` | exec, git via exec, job polling | Gateway services remain issue-time declared and live-authorized; cancellation stays available after revocation. |
| `files` | list/read/write/edit, save/share attachments | Context roots only; optimistic hashes and transfer caps unchanged. |
| `browser` | compact snapshot/actions in secure browser | Node-local browser ceiling; browser-tool support is separate from readiness. |
| `computer` | cua desktop/app tools | Broader desktop authority; in a secure-display context also requires `browser` to avoid bypassing a browser denial through desktop input. |
| `developer_browser` (advanced modifier, default false) | dev browser actions, evaluate/logs/screenshot | Requires `browser`, dev support and separate dev identity/display; never `fill_login`. |

These are tool/API permissions, not magical behavioral sandboxes. Shell necessarily reads/writes its allowed filesystem even if the file-tool switch is off; full desktop control can manipulate apps through their UI. Explain these implications inline. If owners need no terminal execution through a desktop, recommend browser-only access, not `computer`. Scope trust to a context, not to whether the agent selected a particular tool name.

Add an optional node-local `browser` switch, defaulting to the existing `computer` value only when absent, to preserve legacy config. New setup exposes the choice. Server settings can only narrow this ceiling; disabling a local capability always wins. In separated mode, owner control blocks browser/computer input for the selected display and shell/files within that context; another context cannot reach that display. Shared legacy and macOS retain today’s broader shell/files fence. Switching tools or opening another browser in the same controlled context must not bypass takeover.

**NyxBot.** Retain its existing machine reachability for old machines via explicit migrated compatibility grants. For v2 machines NyxBot gets its own context and explicit capability set from owner setup; no automatic access to every specialist's private context. New machine setup may recommend shell/files/browser with clear checkboxes, but does not silently grant them merely by selecting the machine. NyxBot can coordinate work by handing it to the specialist, or ask for owner-approved file transfer. Crossing into another context/browser, granting developer evaluation, widening a capability, switching separated to shared, or adopting a shared login session requires an owner action card. Narrowing/revocation can be done immediately within existing authorized grant-management rules. This preserves the established request-to-NyxBot flow without allowing delegation to bypass the owner's per-capability restrictions. Organization grants still require current org management rights.

**Saved logins.** Keep the existing `saved_login_ids` authority and the B3 prohibition on personal saved logins for organization agents (`docs/ORG_AGENTS.md`); M1 does not introduce organization saved logins. Add a per-assignment optional allowlist that can only narrow it, and bind every fill to the resolved secure context/generation. Absent machine-local narrowing inherits the agent's existing login list; empty means none. NyxBot's existing “all owned logins” behavior remains only in its migrated/shared context; new separated contexts require explicit owner selection. No cookie/profile cloning between agents. Granting the same saved login to two agents intentionally permits both to log into the same external account, but their cookie stores remain separate. Sites themselves can expose shared account/server state; local separation cannot prevent that.

Revoking a login cannot undo remote sessions or data already read. Close and quarantine the affected context's entire secure profile generation before it can be reused; create a clean generation after admission resumes. Origin-only cookie clearing is insufficient for OAuth/SSO domains, service workers and open tabs. Keep workspace data; explain that other logins in that profile need signing in again. Deleting a saved login applies the same reset to contexts that used it, and warns that site-side session revocation may still be required. Maintain a metadata-only usage binding `(login ID, context ID, generation)` to find them. In shared legacy mode, offer an explicit whole-browser reset and state the limitation; never promise per-agent revocation of already-shared cookies.

**Immediate revocation.** Commit a monotonic assignment revision/tombstone and a durable revocation outbox in the same transaction as the grant change. A leased, fenced dispatcher/change stream plus sweep backstop sends a signed context/capability cancel, and the local runtime refuses old revisions, kills affected process groups/cua work, aborts gateway streams and prevents late results. Pending cards bind the revision/context and expire on change. Reductions do not need owner approval and cannot be delayed behind active browser locks.

The current scoped Stop identity is conversation/turn (plus an unscoped machine-wide Stop). Add context/revision/capability cancellation without weakening existing chat Stop or machine-wide emergency Stop. Each running v2 job/stream has a 45-second authority lease, renewed every ten seconds through the server's fresh DB authorization (§8.4); a disconnected node stops that work when the lease expires. V1 legacy work does not acquire this lease. Server APIs stop accepting work immediately; the UI says “revocation pending on offline machine” until acknowledged or locally expired. Normal online delivery targets the current sub-second Stop behavior, but distributed/network failure is never described as instantaneous. Reconnect syncs the current policy before admitting work. Late outbox workers are fenced and cannot restore an older revision.

### 3.3 Inline cards and per-agent activity

**Chat cards.** Extend the existing activity projection with a typed machine receipt: operation kind, node/agent/context IDs, display, job ID when applicable, outcome/error code, timing, and optional encrypted-preview/attachment IDs. Correlate the MCP activity at admission with the signed request and result; only the authenticated server/node outcome can complete it. A JSON `isError` response must be rendered as failed even if HTTP succeeded. Job cards transition from running to completed/cancelled/failed using job state, not an earlier successful `exec` acknowledgement. Late results after Stop/revocation never resurrect a completed card.

Use one compact component family: browser action + thumbnail; file read/written + byte count; command running/completed + exit status and expandable excerpt. Render tool errors with the existing actionable codes. Keep raw commands, filenames, paths, URLs, typed values, DOM text and terminal output out of `TurnActivity`, audit and metadata events. For a useful file card, a sanitized basename or workspace-relative path belongs in the encrypted thread-scoped preview, never a machine-wide log. No automatic persisted command string is needed.

Terminal previews are opt-in per operation/context, capped initially at 2 KiB stdout + 1 KiB stderr and 40 lines combined, UTF-8 normalized, stripped of terminal control sequences, passed through the existing redactor and displayed as plain text. Never preview `fill_login`, clipboard contents, environment dumps or input to protected fields. Redaction cannot recognize every secret a process prints: it is defense in depth, not proof that an arbitrary output is safe. Default to status/exit code and allow the authorized human to reveal bounded output. Store any retained excerpt envelope-encrypted via the attachment storage machinery, with an explicitly supported typed preview format rather than pretending arbitrary text is an image. Tool output remains untrusted data, never instructions or approval.

**Screenshots.** Existing dev-browser screenshot attachments become historical thumbnails with the actual capture time. An owner-opened live thumbnail uses the current memory-only desktop stream and says “Live”; it is not represented as a screenshot of an old action. Secure browser pixels can contain passwords, tokens, private messages and account information; text redaction cannot guarantee their removal. Do not silently persist secure screenshots after every call. Default secure cards to action metadata and an expandable live preview; historical secure capture, if later enabled, needs explicit owner opt-in, capture suppression around login filling/owner input and the normal encrypted attachment controls. No credentials or saved-login field values enter receipt metadata. Reuse one multiplexed desktop subscription per selected context/display; a transcript with 40 cards must not create 40 capture streams or screenshot pollers.

Preview reads use the conversation/attachment ACL, not merely a machine grant. Personal thread previews stay owner-only; an organization node Admin is not entitled to another person's private transcript. Group publication is explicit: only a receipt produced for that group's task can be projected into its shared transcript, under the current participant ACL. Private-context screenshots/outputs cannot be copied there implicitly. Fetches recheck retention and scope; deletion, owner purge and group/conversation cleanup remove preview payloads and render the existing expired placeholder. Tool images retain the admin tool-image policy; introduce a documented bounded retention for non-image previews (proposed 30 days, never longer than conversation lifetime), using the same leased/fenced cleanup architecture. This small schema extension is part of the card PR, not an unencrypted side store.

**Machine activity.** Project the existing metadata audit and jobs into a cursor-paginated machine view, filterable by stable agent ID, context, operation kind, time and outcome. Add agent/actor/context/request/activity/job correlation to new `machine_operation` audit rows; record only bounded enum sub-actions such as `browser.navigate`, not navigation URLs. Join job state by ID so long-running jobs have current outcomes without duplicating output. Start with the existing audit/job stores and indexes; a separate event collection is unnecessary until measured query cost requires it. Use `(node_id, created_at, id)` pagination and an agent-filtered index appropriate to the existing audit schema; bounded default 50/max 100 rows, no unbounded in-memory union. Audit retention remains unchanged. Old rows without agent attribution display “Unknown (older node)”; do not infer an agent from a mutable name or expose an unrelated conversation while trying to fill the gap.

Machine owners and authorized organization machine managers see operation metadata under existing machine-management ACLs. Links to a private thread, previews or command output require their separate thread ACL; omit inaccessible links and conversation titles. An agent sees only its authorized context/task receipts, not a machine-wide activity feed. The page visibly distinguishes `Shared legacy` from `Separated context`, and readiness from capability support. Keep a context picker above the existing Secure browser / Dev browser switch, with takeover/Stop scope clearly named. Machine-wide Stop remains separately available. Follow DESIGN.md and U1 overlay inheritance, keyboard/focus rules, responsive padding and compact transcript width; no per-dialog z-index patches.

### 3.4 Optional per-agent container supervisor

**Recommendation: defer; do not expand the machine updater into a general supervisor.** Today an owner can already run separately paired machine containers and grant each to one specialist. That provides separate PID/network/mount namespaces and independent updates without adding a new host-root API. The context approach improves an existing machine and shares the supervisor/runtime cost, but still shares its kernel and trusted daemon. Separate containers offer a clearer process/network boundary; neither is equivalent to a VM, and neither blocks access to the same public network services by itself.

If demand warrants one-click provisioning later, implement a distinct optional host-side controller used by `nyxid node docker`, never Docker socket access inside an agent job. Docker socket access equals host root, so UI must explain this before installation. Only a signed, owner-confirmed request can create/delete a child for an exact agent/person/group identity. Limit requests to attested official image digests, a fixed container security template, quotas and controller-owned labels/volumes; no arbitrary image, command, path, host network, device, socket or mount supplied by the model. Preserve seccomp, Chromium sandbox, secure/dev user/display separation, trusted updater and identity volumes. Native host services and arbitrary remote Docker endpoints are not guessed or exposed.

Reuse NyxID's existing one-use registration/enrollment for a unique child node identity, retaining revocation and owner/org binding; do not give children the supervisor credential or replace signed node commands with a shared master-derived token. Verify child ownership/labels and the exact expected Docker endpoint before operations, borrowing OpenDots' endpoint-binding discipline. Reconcile creation/deletion under a durable fenced journal, ensure partial failures do not orphan privileged containers, and coordinate child updates with active work. Stopping a child preserves its state; destruction/export is a separate owner action. Grant removal blocks child work immediately and does not silently delete its files.

Cost includes a Chromium/driver/display stack per active computer, persistent profiles and workspaces, image/update storage, context-specific pairing and companion management. Reuse image layers but cap active containers, CPU, memory, pids, disk growth and idle life; show “Paused” separately from “Deleted.” Benchmark before promising capacity. Outbound network policy and an optional preinstalled gVisor runtime are separate future work; do not advertise either without end-to-end support. This phase must not broaden the narrowly scoped, already-attested updater’s socket authority.

## 4. Data model and protocol

Names below are proposed contracts, not fields already implemented. Use UUID string IDs and the existing BSON datetime helpers; dedicated HTTP DTOs, redacted Debug and existing audit append paths remain mandatory.

### 4.1 Durable server state

| Record | Additions / invariant |
|---|---|
| `AssistantAgent` | Sibling `machine_access_version`, `machine_access_revision`, and bounded `machine_access: {node_id: assignment}`. Keep `machine_node_ids` as the coarse membership projection for key/node scopes. An assignment has capability booleans, `mode`, optional narrowing `saved_login_ids`, revision and migration provenance. No caller-supplied context paths/UIDs. |
| Assignment tombstones | Retain monotonic revision after removal; re-adding a node must not resurrect an old command/card. Membership and assignment mutation is one transaction with key synchronization and revocation outbox. |
| `machine_contexts` (new) | Opaque ID, polymorphic agent owner, node/agent IDs, acting-person partition, optional group partition, mode/backend, generation, authority revision, lifecycle and timestamps. Unique index on the identity tuple; node/lifecycle index for reconciliation. No credentials, profile contents, command text or raw filesystem paths. |
| Local context map | Supervisor-owned, no-follow, crash-safe mapping of context/generation to non-reused UID/GID triplets and directories; enforced minimum protocol marker. Provision idempotently, fence concurrent requests, clean partial creations before reporting ready. A restart cannot remap an existing profile to another agent. |
| `machine_jobs` | Context/generation, authority revision and operation capability alongside existing agent/person/thread/runtime/service bindings; extend live gateway authorization, cancellation and lease expiry. TTL of job authority never deletes context data. |
| `machine_desktops` | Context ID in the identity/key, plus display and observation/control epoch. Keep owner session/controller separate from the agent’s task lease. A node-wide emergency Stop iterates all admitted contexts. Existing shared rows map only to the legacy context. |
| Browser task leases | Context/display, holder turn, server-issued fencing epoch and expiry. Atomic acquire/renew; node rejects older epochs even if a stale replica says it owns the lease. |
| Login usage binding | Login ID, context and secure-profile generation, last-used time. Metadata only; revoke/delete can find every generation needing quarantine. No personal login binding for org agents. |
| Revocation outbox | Node/context, capability mask, minimum acceptable revision/generation, reason enum, lease/fence/ack metadata and retry deadline. Durable until acknowledged or superseded by a stronger revision. Coalesce only monotonically. |
| Activities/audit/previews | Correlation IDs and bounded typed metadata as §3.3; encrypted preview bytes in attachment storage. Add explicit preview-kind/retention handling and ACL DTOs, not arbitrary raw tool JSON on message rows. |

Retain the existing specialist machine-grant limit (64 in `apply_grants`) and impose a separately bounded migration for NyxBot’s implicit reachability. If a legacy agent exceeds a chosen new bound, retain its explicit pre-cutover compatibility state and surface it for review; never silently truncate or revoke migrated access. New maps and tombstones must be bounded/compacted safely, retaining revision fences until no old signed request or job can be valid. Batch node/agent/context metadata reads for a page. Keep authorization snapshots request-local; a cached context runtime/model object cannot serve as a cached permission grant. To avoid admitting an old-revision command concurrently with a grant update, admission and revision mutation must share a transactional fence, followed by node-side cancellation/lease enforcement. External effects already completed cannot be undone.

### 4.2 Signed protocol and advertised support

Introduce an explicitly negotiated context/authority protocol version with signed context commands, not optional fields appended to today's generic `parameters` that an old node can ignore. The current machine protocol accepts exactly version 1; negotiate both legacy v1 and the new version deliberately in profile/dispatch code instead of globally bumping a constant and hiding every old machine. Unknown versions/actions fail closed.

A new command authenticates the node/runtime, context/generation, agent/owner/actor/group identity, conversation/turn or job, required capability, grant revision, task/control epoch, bounded authority expiry, request ID and existing nonce/timestamp. Extend signature canonicalization with an explicit version/domain: all authority fields are covered, not separately trusted JSON. The model supplies only the operation arguments. Replayed renewals cannot extend a lease: persist the highest authority/task epoch and derive a bounded monotonic local deadline from the signed expiry; apply expiry even if the socket stays connected but renewals stop. Validate the signed context identity against its local map before opening any file, socket or browser. No untrusted profile selector can cross contexts. Gateway tokens additionally bind the context/revision; effective declared services and B1 checks remain unchanged.

Advertise support separately from current readiness: context protocol versions, enforced backend/platform limitations, capability ceilings, isolation probe result, configured capacity and supported browser actions. Per-context status reports browser/driver readiness, busy/owner-controlled and bounded failure codes. Do not expose another actor's context names or paths in general discovery, and do not empty supported tool lists during driver recovery. `commands_isolated` continues to mean commands cannot read node secrets; add a distinct agent-context-separation fact rather than repurposing it. `browser_isolated` retains its saved-login meaning.

Add typed, retry-aware outcomes for unsupported context protocol, unavailable isolation backend, context busy/capacity, stale authority, permission revoked and context quarantined. Allocate numeric codes only during implementation in `errors/mod.rs`, the authoritative table. Preserve existing driver restarting/permission/display/tool-not-supported/browser-unavailable/Stop codes. Updates and human machine administration retain their own owner-card protocol and are not implicitly granted by `shell` or `browser`.

## 5. Backward-compatible migration and rollout

### 5.1 Compatibility and defaults

Backward compatibility means existing accepted work keeps its explicitly shared semantics, not that old executors may ignore new restrictions.

| Server / node | Permitted behavior |
|---|---|
| Old server + old or dual-protocol node | Existing v1 shared behavior only. An upgraded node blocks v1 for enrolled restricted/separated contexts; an old node cannot enforce them and cannot be used for that policy (see rollback gate below). |
| Upgraded server + v1 node | Unmodified migrated legacy assignments continue to work and show the sharing warning. Switching to restricted/context enforcement requires node upgrade; show an update action, never pretend the old node enforces context leases. |
| Upgraded server + context-capable node | Explicit capability grants, revocation leases and supported context modes enforced at both ends. A Linux probe failure disables separated admission, not legacy readiness. |
| Mixed server fleet | Read-only visibility additions are safe; do not enable new restriction/context writes until every auth, API, MCP, worker, gateway and WS replica supports enforcement. |
| Disabled creation flag after rollout | Existing assignments/contexts/revocations still enforced. Disabling a flag never widens authority or switches a context back to shared. |

Proposed feature flags: `assistant:machine-capabilities` for new assignment configuration and `assistant:machine-contexts` for separated-context setup. Both start off. A separate presentation flag can stage cards/activity without changing authority. Enforcement depends on stored policy, not feature flag evaluation. Node-local enablement and successful isolation probes are required in addition to server flags. Record per-replica supported schema/protocol in readiness diagnostics; require deployment readiness/draining of old replicas before enabling writes, following the B1/B3 rollout pattern.

### 5.2 Ordered migration

1. Deploy additive readers, dual-protocol support and enforcement to the entire server fleet with creation flags off. Preserve the sibling-field write discipline. Install indexes through existing boxed DB phases once, before enabling context writes. Drain old long-lived WS/MCP workers; routing only the settings page to new replicas is insufficient.
2. Run an idempotent bounded, leased/fenced migration of existing agent-machine memberships. Snapshot current compatibility authority as explicit `shared_legacy` assignments, with all historically permitted tool families intersected by local authority. Materialize NyxBot's existing implicit reachability for the person's currently accessible machines (including eligible organization nodes), even if its flat membership list was empty; execution still rechecks live owner/org access. Record a policy cutover epoch and migration provenance; the cutover migration itself never sweeps in post-cutover machines/agents; while the capability flag is off, the live pairing/Grants compatibility path can create their authorized legacy snapshots (§8.4). Serialize migration with grant mutations so it never revives a removal.
3. Set each agent's access-version marker atomically with its migration. Before that marker only the exact pre-cutover compatibility path is allowed. Afterwards absent assignment means deny unless the flag-off NyxBot compatibility path first materializes an authorized legacy snapshot (§8.4); missing fields themselves never grant authority. Both modern and legacy grant endpoints must preserve existing restrictions; adding a `machine_node_ids` entry creates an all-off pending assignment with the editor enabled, or a legacy capability snapshot with it disabled. Reject stale writers that attempt to undo context state. Surface only the capabilities the node can actually enforce.
4. Upgrade nodes through the existing attested updater flow; do not manually replace its trust or hand-off protocol. An upgrade alone does not opt an owner into separated workspaces or move files. New compatible installs recommend separated mode; initial capability toggles stay off until explicitly confirmed. Old and single-user nodes keep the honest legacy warning and upgrade/separate-container guidance.
5. Owner opts an assignment into separated mode using a reviewable card/settings action showing root change, clean browser, capacity and effects on running work. Drain/cancel its current jobs and desktop input, provision a fresh context, verify enforcement, then commit the new assignment revision. On failure retain the old assignment only with its old explicit policy; never execute a separated request in the old workspace as a fallback. Existing agent tasks may continue in shared mode until their owner elects migration.
6. Do not clone legacy cookies, native sockets, credentials or an entire HOME. Offer explicit selected workspace-file transfer using existing validated file/attachment paths, scan/reject symlinks/special files and exclude caches/secrets; copies are owner-reviewed and do not imply inherited execution permission. The old shared workspace/profile remains available to explicitly shared assignments. Reauthenticate approved saved logins in each new secure context.

**Opt-out/reset.** Separated to shared is an explicit authority widening: owner confirmation, stop that context, increment revisions, and state that other agents may use the shared files/sessions. Do not merge private context data or cookies into the shared profile. Retain separated data for explicit export/destruction. Revocation and deletion are distinct from opt-out. Existing machines default shared; new supported setups recommend separated, with consent and deny-by-default capability selection. Never surprise an existing machine with an empty workspace after an automatic update.

### 5.3 Rollback and local lifecycle

Once a restricted/separated assignment exists, rollback to an old server or node binary is not a transparent operation. Disable new creation, stop/drain affected turns/jobs/streams, revoke thread/job credentials as required, and keep context/preview/history routes on compatible replicas while their data needs new ACLs. Old binaries must not serve new private group/context previews. Retain tombstones and local minimum-protocol markers; old signed commands cannot restart work. Node automatic rollback may restore availability for legacy contexts but must keep restricted contexts blocked pending a compatible version. An explicit owner choice to return to legacy is a separate reviewed migration, not the updater's failure fallback.

Use the existing persistent state volume for context maps/profiles and preserve it through container recreation. A node-local v1-unaware binary may not enforce a new marker itself: before shipping context mode, the update admission/rollback controller must refuse to start such a binary against enabled contexts, and the upgraded server must refuse every operation to that runtime. Do not claim a marker unread by an old binary is a security boundary. Rollout acceptance includes attempted downgrade and replay, not just happy-path forward updates.

Agent destruction, actor membership loss and group participant removal enqueue context cancellation and cleanup/quarantine appropriate to the existing B3 rules. Never reassign abandoned context data to another person. Owner purge removes server previews and queues local erasure; offline data remains marked pending until acknowledged. Preserve the local UID mapping until every process and owned file is gone, so UID reuse cannot expose retained browser state. This is local lifecycle integration, not a new promise to erase external sites or remote service data.

## 6. Test and measurement plan

This is the acceptance plan for implementation PRs. No runtime, container or performance tests were run for this source-only design audit.

| Area | Required checks |
|---|---|
| Existing behavior | Legacy shared machine still executes its previously allowed operations; v1 visible; saved-login, owner controls, driver recovery, updater and B1 suites remain green. Existing assignments do not silently change roots or lose browser tabs. |
| Capability authority | Every capability on/off combination, local ceiling off, false/absent/defaulted fields, forged assignment/context, destroyed/recreated agent, guest, mismatched owner/node, expired job and missing live turn. Browser denial cannot be bypassed with cua; shell/file permission implications documented and tested. Saved-login/dev and org-personal-resource prohibitions preserved. |
| B1/B3 | Native machine tools remain native rather than catalog service operations. Final gateway calls preserve service operation scopes, grants, membership/platform ACL and person billing. Two members using the same org agent, two groups and private/group tasks cannot read each other's contexts, cards or outputs. Membership and participant removal stop new work and cancel affected running authority. |
| Linux adversarial isolation | Two specialists plus legacy agent, hostile shell and browser process under each role UID: absolute/relative traversal, rename/symlink/hardlink, inherited fd and `/proc/.../fd`, same-name recreation, credential/config/store read and write, cookies, D-Bus, CDP/native sockets, Xauthority, process signal/ptrace and shared temp access. Verify actual failure, not just mode bits. Sibling profiles/displays and legacy roots remain inaccessible even with world-readable files or writable parent tricks. |
| Unsupported platforms | Missing Landlock/required ABI, restricted user provisioning or broken ownership probes return separation unavailable; no fallback. macOS/single-user labeled accurately. An unavailable separated backend must not hide valid legacy tools or break machine update. |
| Revocation and races | Remove shell during foreground/background exec and gateway streaming; browser/computer during long action; files during transfer. Verify correct scope, no surviving child process groups, blocked old revisions and late replies, unrelated context continues. Exercise two replicas, stale outbox holder, rollback/re-add, duplicate/reordered messages, disconnect and authority expiry; reconnect cannot resurrect cancelled work. Revoked login quarantines the right generation, including SSO tabs. |
| Context lifecycle | Concurrent provisioning, failure at each local-map write, daemon/container restart, idle eviction, capacity, agent deletion/purge, never-reused UID and local-data export/cleanup. Owner takeover and Stop names the context/display, works during browser locks, and invalidates stale observation refs on handback. |
| Browser regression | Trusted clicks/activation, overlay hit test, cross-origin frames, protected fields, 500-row paging, secure DevTools disabled, dev evaluation working. Persistent-profile extension reconnect/package repair, secure-browser kill/relaunch, container restart/update, missing package and read-only machine status in at least two simultaneous contexts. Both displays stream for each selected context without cross-input. |
| Cards/activity/privacy | Typed failure despite HTTP 200; async job completion, Stop and discarded late outcome; bounded excerpts, terminal escapes, secret fixtures and safe filenames. Owner/other owner/guest/other agent/thread/org Admin/group ACLs; current retention after shortening, deletion/purge and expired placeholders. No content in logs/audit/live identifiers; no accidental private-preview publication to group. One live capture subscription, bounded list queries, pagination and old-attribution placeholders. |
| Rollout | Mixed replicas cannot author new enforcement state; migrated marker/absent entry denies; old grant writer cannot restore unrestricted access. Test old node rejecting new protocol, wrong signature version and downgrade/rollback blocking, with persisted context volumes and old command replays. |

Container correctness runs use the shipped seccomp profile **and** the mount-denying AppArmor approximation, plus an actual Docker AppArmor host where available. Run Linux arm64 locally and amd64 in CI; native `--separate-users` tests cover computer disabled as well as enabled. Do not infer AppArmor compatibility from macOS Docker Desktop alone. No added capabilities, privileged mode or `apparmor=unconfined` to make isolation tests pass.

Follow existing CI policy: print timing measurements and use generous sanity ceilings (desktop input-to-frame p95 ≤500 ms and a floor high enough to reject the old 5 fps path, e.g. ≥8 fps). Use deterministic correctness synchronization plus bounded waits for Stop/revocation, rather than asserting sub-second timing on shared runners. Quiet-host benchmarks separately report strict D17 ≥15 fps / p95 ≤100 ms and current sub-second Stop/takeover goals. For offline revocation test the logical lease deadline with controlled time; live wall-time runs report network and scheduler variance.

Measure 1/2/4 active contexts: cold/warm browser start, snapshot/click/type/cua AX latency, CPU/RSS, disk per profile/workspace, stream bandwidth and per-context fairness. Compare the same local multi-step web fixture, same image/driver/host, call count and wall time, before/after. Measure grant-revocation delivery and cancellation end-to-end, not just HTTP return. The earlier machine-fixes figures (6 calls/251 ms browser versus 18 calls/15 s cua; 30 fps/38 ms p95 desktop) are historical user-verified results, not measurements from this audit or promised context overhead. Context limits remain provisional until this benchmark is recorded.

Each implementation PR runs fmt, workspace clippy, focused backend authority/machine/job/gateway/card/retention tests at default stack and `RUST_MIN_STACK=1572864` (box large futures), appropriate CLI/machine/extension tests and frontend tests/lint/type-check/build. Full machine/container/updater e2e is required when runtime, persistent layout or update compatibility changes. Use private Mongo on 27020, keep target below 35 GB, stop builds below 15 GB free, and remove only this task's test containers/volumes/images/caches. This design phase needs none of those resources.

## 7. Shippable PR phases

| PR | Shippable result | Gate / dependency |
|---|---|---|
| M1.1 — Visibility and receipts | Honest shared-session badge, action-specific metadata cards, existing screenshot thumbnails, bounded encrypted previews, metadata activity page with agent filter; typed failure/job outcomes. Reuse desktop and attachment retention. | No new machine authority. ACL, privacy, retention, frontend responsiveness and bounded-query tests. Can ship on legacy nodes; unattributed history stays unknown. |
| M1.2 — Capability authority | Explicit per-agent machine capability editor/tool cards; compatibility migration; dual protocol, signed revisions, context identifiers, revocation outbox and authority leases. Initially operates labeled shared contexts on upgraded nodes. | Whole-server rollout before configuration flag; capable nodes required for new restrictions. Older nodes retain only migrated legacy mode with upgrade guidance. Revoke/Stop/gateway/B1/B3 and mixed-fleet tests pass. |
| M1.3 — Separate workspace and browser (revised) | Opt-in separated shell/files and secure/dev browsers, durable UID map, protected roots, runtime probes, context lifecycle and profile quarantine. | Revised boundary in §3.1; adversarial controls and two simultaneous browser contexts. No shared fallback. |
| M1.4 — Folded into revised M1.3 | Browser separation and handback are implemented with the workspace boundary, per the owner decision in §8.6. | Full isolation still requires a separate container/VM per agent. |
| M1.5 — Optional container provisioning (decision gate) | A separately approved Docker supervisor with fixed templates, unique child identities and per-child lifecycle. | Not necessary to complete M1.1–4. Proceed only if owners need stronger namespace separation/one-click fleet creation and accept the host-root companion plus resource cost. New security review and attested lifecycle tests. |

Each PR includes its migration, docs and tests; disabled creation flags leave a complete backward-compatible release. Do not combine all context, browser and supervisor changes into a single mandatory upgrade. If the OS sandbox spike cannot meet the threat model on supported hosts, ship capability/visibility improvements and recommend separate existing machine containers; return for a decision rather than weaken the meaning of “isolated.”

## 8. Decisions, limitations, and evidence index

### 8.1 Approved decisions (2026-10-03)

1. Direction and order approved: M1.1 → M1.2 → M1.3 → M1.4, each its own PR. M1.5 is deferred; existing separate machine containers remain the recommendation for stronger separation.
2. Existing machines remain `shared_legacy` with the visible warning. New capability assignments are deny-by-default when the acting person has the editor enabled. The M1.2 review clarifies that flag-off owners retain legacy pairing and Grants semantics through explicit snapshots (§8.4); toggling the flag never widens an existing restriction. NyxBot gets its own context on v2 machines. Organization-agent contexts partition per acting person and additionally per group for group tasks. `assistant:machine-capabilities` and `assistant:machine-contexts` default off; stored restrictions are enforced independently of those flags.
3. M1.3 acceptance depends on the Landlock spike meeting the threat model on supported hosts. If it cannot, stop and report back. Never weaken the meaning of “isolated”.
4. The HTTP-200/MCP-`isError` activity bug is general. M1.1 fixes it for all tools using the typed MCP outcome, with regression coverage.

M1.1 adds visibility only: shared-mode labeling, typed receipt cards, opt-in bounded encrypted command previews, and a metadata-only machine activity view. Preview retention is at most 30 days, shortened by the current document retention policy; disabling previews prevents new capture. No context/capability authority is introduced in this PR. Future phases retain all existing B1/B3, owner-confirmation, update and billing contracts. Resource limits and the precise isolation backend remain implementation acceptance gates.

### 8.2 Source index

Source links below are pinned to the merged NyxID baseline or the audited OpenDots revision; findings above use symbols and initial-audit line anchors to make the comparison reproducible. The OpenBot supervisor implementation was not independently audited: claims about its security and lifecycle here are limited to the pinned OpenDots integration and its documented contract.

| Evidence | Relevant contract |
|---|---|
| [Current machine design/threat model](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/docs/MACHINE_NODES.md) | D18 and machine/browser/container sections; existing primitives to preserve. |
| [Agent grant shape](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/backend/src/models/assistant_agent.rs#L97) | AssistantAgent: sibling machine/login grants, owner and specialist identity. |
| [Machine authorization and jobs](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/backend/src/services/machine_service.rs#L86) | Guest refusal, visible nodes, granted, job issuance/polling. |
| [Machine tools and audit](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/backend/src/handlers/machine_tools.rs#L130) | Owner control, fill_login, live turn and metadata-only machine_operation audit. |
| [Grant mutation](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/backend/src/services/assistant_team_service.rs) | apply_grants and authority synchronization; no machine cancel dispatch on grant removal. |
| [Gateway live authority](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/backend/src/handlers/machine_gateway.rs#L554) | job_auth and final execution gates. |
| [Node singleton runtime](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/cli/src/node/machine/runtime.rs#L149) | Runtime, admission/control checks and cancellation selection. |
| [Filesystem checks](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/cli/src/node/machine/files.rs#L129) | Roots and descriptor-relative no-follow operations. |
| [Process boundary](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/cli/src/node/machine/process.rs) | prepare_agent/identity, env clearing, setuid/gid and seccomp. |
| [Secure/browser display state](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/cli/src/node/machine/browser.rs#L374) | Singleton socket/profile/lock and extension/native-host ownership. |
| [Developer browser](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/cli/src/node/machine/dev_browser.rs#L116) | Profile/identity, CDP and native/container launch behavior. |
| [Container layout](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/cli/container/entrypoint.sh) | Separate secure/dev Xauthority, DBus and user startup; Dockerfile.machine supplies users/volumes. |
| [Desktop metadata](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/backend/src/models/machine_desktop.rs) | MachineDesktop fields; session identity currently node/display in machine_desktop_service. |
| [Activity record](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/backend/src/models/assistant_conversation.rs#L81) | TurnActivity/ActiveTurn; bounded activities in assistant_nyxagent. |
| [MCP outcome recording](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/backend/src/handlers/mcp_transport.rs#L1533) | Activity start/finish around boxed native dispatch. |
| [Org privacy rules](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/docs/ORG_AGENTS.md) | Actor-owned threads, org resources, explicit group participants and hidden actor threads. |
| [B1 flag precedent](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/backend/src/services/agent_operation_scope_service.rs#L96) | Configuration gating separate from always-on enforcement. |
| [Attachment lifecycle](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/backend/src/services/assistant_upload_retention.rs) | Current-policy reads, fenced sweep and tombstone placeholders. |
| [Existing live desktop UI](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/frontend/src/components/assistant/machine-desktop-panel.tsx) | Stream/control/secure-dev switch/collapse/fullscreen/pop-out. |
| [UI conventions](https://github.com/ChronoAIProject/NyxID/blob/36e3ef04b571f80bafab0fee88cc2b77ac5abaf8/DESIGN.md) | Transcript grid and U1 inherited overlay layers. |
| [OpenDots computers](https://github.com/CopilotKit/OpenDots/blob/c2569bb6a13a22e565cf3eb791c62267d06babb1/docs/COMPUTERS.md) | Per-Dot lifecycle, default-disabled capabilities and explicit isolation/cancellation limits. |
| [OpenDots deployment](https://github.com/CopilotKit/OpenDots/blob/c2569bb6a13a22e565cf3eb791c62267d06babb1/deployment/computers/README.md) | Pinned OpenBot source and scoped token patch. |
| [OpenDots authorization](https://github.com/CopilotKit/OpenDots/blob/c2569bb6a13a22e565cf3eb791c62267d06babb1/src/server/computer-service.ts) | Scoped HMAC token, endpoint binding, permission polling and request cancellation. |
| [OpenDots store](https://github.com/CopilotKit/OpenDots/blob/c2569bb6a13a22e565cf3eb791c62267d06babb1/src/server/computer-store.ts) | Default-denied capabilities and bounded metadata audit. |
| [OpenDots tool contract](https://github.com/CopilotKit/OpenDots/blob/c2569bb6a13a22e565cf3eb791c62267d06babb1/src/server/computer-tools.ts) | Capability-aware actions, snapshot refs and fresh observation after handback. |
| [OpenDots panel](https://github.com/CopilotKit/OpenDots/blob/c2569bb6a13a22e565cf3eb791c62267d06babb1/src/client/ComputerPanel.tsx) | Browser/Files/Terminal/Activity organization. |
| [OpenDots cards](https://github.com/CopilotKit/OpenDots/blob/c2569bb6a13a22e565cf3eb791c62267d06babb1/src/client/ComputerToolCard.tsx) | Action-specific excerpts and current-screen polling, not historical screenshots. |

Initial design-audit validation used pinned-source inspection and cross-file call-path review only. M1.1 implementation and its validation are tracked separately below; later phases remain design-only.

### 8.3 M1.1 implementation notes

M1.1 keeps machine admission, grants, signed commands and node runtime unchanged.
Receipts are boxed optional metadata on the existing bounded activity rows.
MCP response construction attaches a typed outcome for JSON and SSE; recording
never infers success from HTTP 200. Each dispatched machine operation receives
an operation UUID and inherits the outer MCP activity UUID, including universal
`nyx__call_tool` calls. New audit rows add agent/job attribution; old rows remain
unattributed. Files report actual read/write bytes when available, not the total
source file size; old edit results without a count stay unknown.

The human conversation owner alone can opt into future encrypted command
excerpts. Each capture rechecks the exact live turn, consent, non-guest/non-group
scope and Stop fence transactionally with the attachment reference. Disabling
does not delete old captures. Excerpts use `machine_preview` attachment origin
and expire after `min(document_days, 30)` from creation. The existing fenced
sweep and conversation/owner deletion paths apply. Full replica upgrades precede
capture enablement, as with upload retention changes. Group publication strips
private preview and screenshot references; group cards are metadata-only even
when a private thread previously enabled captures. Job state is joined in bounded
batches on transcript and activity reads, without giving a viewer job execution
or private thread access. Terminal output is loaded only on explicit reveal.

Activity pages require a first-party human owner or organization admin, return
only a fixed metadata projection, and use compound indexes, a 100-row maximum,
3-second query deadlines and cursor pagination. Agent IDs filter current audit
attribution; they do not grant access. Command excerpts remain inaccessible to
organization admins viewing somebody else's private thread.

The M1.1 activity picker returns one bounded batch of the viewer's personal
agents and this machine's organization specialists, after the machine-owner
ACL. It never resolves another member's private agent names. Machine names are
read-time DTO enrichment using the existing node read ACL, batched once per
transcript page (including group history) and never written into audit rows.
Raw identifiers stay inside collapsed correlation disclosures; unattributed
activity has an explicit “Unknown (older node)” filter.

### 8.4 M1.2 capability authority

M1.2 adds an explicit boxed `machine_access` sibling policy (version, monotonic
editor revision, per-machine execution revisions and assignments). For acting people
with `assistant:machine-capabilities` enabled, new selections start with all
capabilities off and the editor is available. With the flag off, existing workflows
keep working: NyxBot snapshots missing assignments at discovery or first use after
live ACL checks, and the specialist Grants picker snapshots selected machines.
Both use `legacy:true` and the node's current legacy capability profile. The
one-time cutover likewise snapshots pre-existing reachability as `shared_legacy`.
Turning the flag on preserves these snapshots until edited. Turning it off never
widens an explicitly configured assignment.
Migration and grant writes serialize on the agent row. Removal and re-addition
retain the global revision fence rather than growing an unbounded tombstone map.
Legacy rosters above the 500-machine discovery page remain explicit and usable;
migration streams all pre-cutover rows in batches of 100. The editor limits new
assignments to 64 without truncating or revoking larger legacy rosters.

Profiles retain wire version 1 for old servers and separately advertise authority
version 2. Agent requests to capable nodes use the distinct `machine_request_v2`
message and signing domain. The signed envelope binds the server-derived opaque
context, agent/owner/acting person/group, runtime, turn, revision, capabilities
and 45-second authority lease, renewed every ten seconds. Context IDs are persisted UUIDv4 values behind a
unique node/owner/agent/person/group identity index. These context IDs partition authority only in
M1.2: files and browser sessions remain visibly shared. No separated mode is
accepted until its later enforcement work is complete.

Capability writes and revocations append a transactional outbox. A change stream
wakes per-row fenced delivery; the leased one-second sweep retries missed events, including to another replica's socket; monotonically increasing
node fences make late delivery harmless. Live leases reauthorize from MongoDB,
including current thread keys, actor access, grants and background job state.
The local monotonic watchdog cancels expired work and gateway streams after a
network loss. Old nodes get only their existing scoped Stop command, with persisted cursor
progress across bounded batches, and preserve unmodified legacy access; explicit capability edits require an upgrade. A node
that accepts a restricted assignment durably refuses v1 agent work. Human Stop,
control and attested updates retain their separate authority path.

The editor and `nyxid__machine_capabilities` expose local ceilings, inherited
legacy access and the required revision. Widening through an agent requires an
owner action card. `assistant:machine-capabilities` gates editor writes and selects defaults for new
assignments; it cannot bypass stored restrictions. Both machine flags default off. Deploy all server
replicas before enabling configuration. Rolling back to a server that ignores
these restrictions requires draining affected work and explicit operator review;
never serve restricted assignments through a v1 node.

M1.2 resource bounds: admission is transactionally capped at **128 live v2
operation leases per node**, above the node's maximum 64 local jobs (default 4).
There is no fleet-wide lease cap. Idle connections, expired rows and other nodes
do not count. Saturation returns HTTP 429 `machine_authority_busy` before dispatch:
retry when an operation finishes. Running work and renewal continue, and job
cancellation bypasses admission capacity. V1 nodes return before lease admission
and are never affected by this cap. Renewal streams bounded batches with 64
concurrent requests every ten seconds, independently of the one-second revocation
sweep and compatibility migration. A brief socket loss preserves the running job;
reconnect can renew it while the last 45-second signed lease is still live.
Push revocation remains immediate, with the offline deadline as its backstop. Lease messages are never retried past an
expired local deadline. The compatibility snapshot uses bounded driver batches, without applying
new-assignment limits to existing access. Node fence storage is private, capped at 2 MiB and
persisted before activation. Context metadata remains separate from process or
filesystem isolation.

### 8.5 M1.3 feasibility gate (2026-10-04)

The Linux workspace spike did **not** pass the approved acceptance gate. The
tested UID/Landlock/NNP/seccomp combination permits outside metadata writes and
blocks ordinary Python process pools when shared `/dev/shm` remains inaccessible.
The original full-isolation proposal was stopped; §8.6 records the revised decision. See [MACHINE_CONTEXT_SPIKE.md](MACHINE_CONTEXT_SPIKE.md)
for counterexamples, ABI/kernel requirements, tooling results and reproduction.
The meaning of `isolated`, the shared-legacy default and the default-off context
flag are unchanged; stronger enforcement requires a new design decision.

### 8.6 Revised M1.3 decision (2026-10-04)

The owner chose **Separate users + browsers** after reviewing the failed original
spike. M1.3 now combines the former M1.3 and M1.4, with the narrower `separated`
boundary in §3.1. The original failed gate is historical, not a claim that its
counterexamples have disappeared. M1.5 remains deferred; separate machine
containers/VMs remain the recommendation for full isolation. No existing machine
or assignment changes mode without owner opt-in.

### 8.7 Backend/runtime review slice

This slice implements signed context selection, durable UID allocation and
generation quarantine, workspace enforcement, separate secure/developer browser
resources, context-bound login fills and the native owner-card opt-in flow. The
existing capability form and historical tool cards show the stored mode without
calling it isolated. The context flag remains off by default.

Provisioning is lazy on the first signed operation after the owner card changes
the policy. If provisioning fails, that assignment remains separated and refuses
work; it does not automatically restore shared access. A future preflight UI can
prepare the resources before completing the visible setup flow.

The human context picker, graphical opt-in flow and context selection in the live
desktop HTTP/WebSocket API and UI are still follow-up work before the complete
feature ships. The runtime routes signed context desktop commands and applies takeover to that context
(including only jobs started under its supervisor-selected command UID), but
the current human desktop API and page still select the legacy desktop. Do not
enable this review slice for owners before that routing and selector are complete.
A separate native Linux VM browser run remains part of release validation; container evidence and native
policy ACL unit tests do not substitute for that host check.

### 8.8 Runtime validation evidence (2026-10-04)

The revised adversarial harness passed **44 checks on each seccomp profile** on
native arm64 LinuxKit 7.0.14 (Landlock ABI 8): the shipped profile and a profile
that additionally denies mount, umount2 and pivot_root. The controls explicitly
permit the documented system-metadata, public pathname-socket and shared-memory
residuals; these are not isolation claims. Git, Python Process/Pool and venv, Node,
npm, cargo, C/C++/make and pip ran under the command filesystem ruleset.

The complete machine-container e2e passed both profiles, using the production
launcher and compiled runtime. The added context cases exercise two workspaces,
six distinct role UIDs, four simultaneous display streams, private AT-SPI trees,
secure/dev profile and socket denial, mutual X authentication refusal, scoped
running-job takeover, saved-login binding and generation quarantine. Entrypoint
filesystem initialization preserves the legacy DAC gate. Existing shared-browser
relaunch, repair, trusted-input, driver recovery, Stop and v2 revocation checks
remain in the same run. CI sanity ceilings were used; these busy-host runs are
not strict quiet-host performance benchmarks.

An earlier run produced a transient generic error during first secure-context
navigation; the subsequent full runs passed. No root cause is claimed for that
observation. Secure native-transport failures now have fixed typed read/write,
timeout and invalid-response diagnostics (12413), with no page content in logs.
The earlier one-off context file-write error also did not recur; the fixture
includes bounded workspace metadata on failure. Repeat this matrix in CI and on
a native separate-users Linux host before enabling the feature for owners.

### 8.9 Backend/runtime validation (2026-10-05)

Rust tests used 1.98.1 at the default stack and `RUST_MIN_STACK=1572864`.
The backend used the private MongoDB replica set on port 27020 only. Counts
below are per stack; backend filters overlap and should not be summed as unique
tests.

| Suite/filter | Default stack | Reduced stack |
|---|---:|---:|
| Host CLI + machine | 1,506 passed, 6 ignored | 1,506 passed, 6 ignored |
| Linux arm64 CLI + machine | 1,511 passed, 4 ignored | 1,511 passed, 4 ignored |
| Linux privileged context/ACL checks | 5 passed | 5 passed |
| Backend `machine` | 99 passed, 3 ignored | 99 passed, 3 ignored |
| Backend `saved_login` | 6 passed | 6 passed |
| Backend `chat_authority` | 24 passed | 24 passed |
| Backend `assistant_team` | 12 passed | 12 passed |

The privileged Linux checks were executed separately as root; the ordinary
suite runs as an unprivileged user. One reduced-stack backend run hit the test
helper's MongoDB reachability deadline before entering a test's assertions. The
unchanged filter passed on rerun; neither stack sizes nor deadlines were raised.

Frontend validation passed **4,226 tests in 427 files**, lint (zero errors,
29 existing warnings), and the production build. Wizard freshness passed; the
wizard source and bundle were not changed. The adversarial and complete native
arm64 container results are recorded in §8.8. Native separate-users VM browser
validation and the missing owner desktop/opt-in surfaces remain release gates
in §8.7.

`cargo +1.98.1 clippy --workspace --all-targets -- -D warnings` passed on
the host; Linux arm64 CLI/machine Clippy passed with the same toolchain and
warning policy. Workspace fmt and `git diff --check` passed. Test containers,
images and the dedicated builder/cache were removed; `target/` remains about
8 GiB. No versions or commits were created for this review slice; the main merge
remains uncommitted.

### 8.10 Review corrections and shipping decision (2026-10-05)

The owner approved shipping this backend/runtime milestone with
`assistant:machine-contexts` still default-off. Graphical opt-in, the human
desktop API/context selector and native separate-users VM browser validation
continue as **M1.3b on the same branch after this milestone merges**. The rollout
restrictions in §8.7 and the 12407 diagnostic observations in §8.8 still apply.

The latest main (`d9a53135`, v0.58.0) was merged without committing, preserving
agent learning, its flags, conversation fields and enrollment snapshots. The
optional mode now belongs to `machine_capabilities.selection`, while the
`request_agent_skills` schema matches main. Validation also accepts the already
advertised nullable saved-login list. Node-advertised support accepts future
fields and defaults missing support to denied; durable local context records
remain strict. Shared developer-browser launches retry a missing cached user;
separated contexts never substitute that shared identity. CLAUDE.md documents
the authority codes and the owner-card-only separated-mode boundary.

Review revalidation used Rust 1.98.1, `CARGO_INCREMENTAL=0`, the default
stack and `RUST_MIN_STACK=1572864`. Counts below are per stack; backend filters
overlap. Every listed run passed without retries or stack/deadline increases.

| Suite/filter | Default stack | Reduced stack |
|---|---:|---:|
| Host CLI + machine | 1,509 passed, 6 ignored | 1,509 passed, 6 ignored |
| Linux arm64 CLI + machine | 1,515 passed, 4 ignored | 1,515 passed, 4 ignored |
| Linux privileged context/ACL checks | 7 passed | 7 passed |
| Backend `machine` | 101 passed, 3 ignored | 101 passed, 3 ignored |
| Backend `saved_login` | 6 passed | 6 passed |
| Backend `chat_authority` | 24 passed | 24 passed |
| Backend `assistant_team` | 14 passed | 14 passed |
| Backend `assistant_agent_learning` | 3 passed | 3 passed |

The Linux privileged checks include creating the shared developer-browser user
after runtime startup and resolving it without a daemon restart. The wire tests
cover unknown node support fields, missing support defaulting to unavailable,
and strict durable records. Frontend validation passed **4,226 tests in 427
files** and lint (zero errors, 29 existing warnings). Wizard freshness passed.

Host workspace/all-targets and Linux arm64 CLI/machine Clippy passed with
`cargo +1.98.1` and `-D warnings`; workspace fmt and staged/unstaged diff checks
also passed. The private MongoDB container and dedicated Linux builder/cache
were removed. `target/` stayed below 10 GiB and free disk stayed above the
15 GiB cutoff. No commit or additional version bump was made; main's v0.58.0
version files remain unchanged and the merge is uncommitted.

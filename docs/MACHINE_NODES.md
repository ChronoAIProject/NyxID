# Machine nodes: NyxBot and agents using the owner's machines

This document describes the machine-node contract, setup and operation.
[Validation and measurements](#validation-and-measurements) maps acceptance
criteria to automated coverage and gives repeatable performance commands.

## What the user asked for

> extend the credential node that we have, such that user can install it easily
> either on host, remote VM or container, so … nyxbot or/and specialist agents
> can use that to access the user's machine to help with stuff like coding,
> computer use etc, best is https://github.com/trycua/cua the cua driver … we
> recommend user to install on a remote machine or container that is not their
> own personal one to be safe and they can install multiple node as before …
> nyxbot or agents can fix or modify files for either coding or docs etc, or image
> generation, git clone from github etc as long as user have those services
> connected.

The credential node (`nyxid node`, `cli/src/node/`) keeps everything it does
today (proxying, node-held credentials, SSH). It gains **machine access**: the
owner of a node can let their NyxBot and chosen specialist agents run commands,
read and change files, use git with the owner's connected services, and operate
the desktop through the cua driver, on that machine.

Nothing here may break existing nodes, proxying, SSH, NyxBot or specialists.

## Decisions

### D1. Capabilities and terms

A node's machine access has three independent capabilities:

| Capability | What agents can do |
|---|---|
| `shell` | Run commands (foreground or as background jobs) and use git |
| `files` | List, read, write and edit files, move files between the machine and the conversation |
| `computer` | Operate the desktop (screens, windows, apps, input) through the cua driver |

User-facing name: **machine** ("Let agents use this machine"). A node with no
capability enabled behaves exactly as today.

### D2. The machine's owner opts in on the machine (node-local authority)

The node's local configuration is the authority for what the machine exposes.
Nothing the server sends can enable a capability.

- New `[machine]` section in the node config (per `--profile`):
  - `shell`, `files` and `computer` booleans, default off;
  - `roots`: one or more directories the file tools and command working
    directories are confined to. The default is a dedicated workspace directory
    the enable command creates, `~/nyxid-workspace` (container:
    `/workspace`);
  - `computer_mode`: `standard` (default) or `unrestricted` (cua's mode, D7);
  - limits with safe defaults: max concurrent jobs (4), max command timeout
    (3600 s), output caps.
- CLI (all accept `--profile`):
  - `nyxid node machine enable [--shell] [--files] [--computer] [--root DIR]... [--computer-mode standard|unrestricted] [--allow-root]`;
  - `nyxid node machine disable [--shell|--files|--computer|--all]`;
  - `nyxid node machine status`: what is enabled, roots, cua driver version
    and permissions (macOS Screen Recording/Accessibility), and warnings.
  - `enable` with no capability flag enables `shell` and `files`.
- The node refuses to enable `shell` or `computer` while running as root unless
  `--allow-root` is given, with a warning.
- The node advertises its machine capabilities through the existing node
  capability reporting (`NodeCapabilitiesMsg`, `node_owner_service::record_capabilities`):
  enabled capabilities, OS, arch, root display names, cua driver version,
  computer mode and the cua tool list. Changes take effect when the daemon
  restarts or reconnects.
- The server stores the advertised machine profile on the `Node` (additive,
  serde-defaulted). It only ever narrows: a request for a capability the node
  did not advertise is refused server-side, and again node-side.

### D3. Who may use a machine (server-side authority)

- **Callers:** the owner's NyxBot and specialist agents, meaning their assistant
  chat keys (NyxAgent conversations, including group member threads), on turns
  started by the owner. That is the whole audience the user asked for. Guest
  turns (`ChatAuthority.guest`) never get machine tools, at any guest access
  level: machine access is the owner's, like SSH. Other API keys, delegated
  tokens, relay tokens and service accounts get no machine tools.
- **Which machines:**
  - NyxBot may use every machine node the owner can use;
  - a specialist may use only the machine nodes granted to it;
  - "can use" = the node's owner is the agent's owner, or the node is org-owned
    and `org_service::resolve_owner_access(owner, node.user_id)` gives the owner
    write access (org admins);
  - every call re-checks this live, together with the node being online and
    advertising the capability.
- **Specialist grants:**
  - a new `AssistantAgent.machine_node_ids`, stored **beside** `grants`, not in
    it, for the same rolling-deploy reason as `guest_access`: replicas that
    predate it rewrite only `grants`;
  - `nyxid__spawn_subagent`, `nyxid__grant_subagent` and `nyxid__revoke_subagent`
    accept `machines` (node names or IDs); `nyxid__list_subagents` and the
    agent summary show them;
  - the agent page's Grants form has a machine picker;
  - `PUT /agents/{id}/grants` accepts an optional `machines`, and left-out means
    unchanged;
  - a specialist calling an ungranted machine gets the existing permission
    request flow (`decider: orchestrator`), new kind `machine`, decided by
    NyxBot (`nyxid__decide_permission`) or the owner, never granted
    automatically.
- **Owner confirmation per node:** a server-side per-node setting
  `machine_confirm`:
  - `none` (default): no confirmation;
  - `changes`: every operation that changes the machine (exec, write, edit,
    git fetch/pull/push/clone, file save, computer actions other than pure
    observation) needs a single-use action card first, reusing the existing
    action-card/acknowledgement machinery of destructive account tools. In chat
    apps it is decided with the card's 4-digit code, as today;
  - `all`: reads need a card too.

  Only the owner changes it, on the Assistant → Machines page, through a human-only route.
  NyxBot cannot change it; it can hand out the settings link.

### D4. Tools

These are native MCP tools, listed and callable only for chat keys allowed by
D3. They are discoverable through `nyx__search_tools` and described so the
model knows when to use them. Names:

| Tool | Purpose |
|---|---|
| `nyx__machine_list` | Machines the caller may use: name, id, status, OS, capabilities, roots, computer mode, confirmation setting |
| `nyx__machine_exec` | Run a command. `{machine, command, cwd?, services? (explicit slugs or IDs; default empty), env?, stdin?, timeout_secs? (default 120, max node limit), background? }`. Foreground returns `{exit_code, stdout, stderr, truncated, duration_ms}`; background returns `{job_id}` |
| `nyx__machine_job` | `{machine, job_id, wait_secs? (≤ 60), output_offset?}` → status, exit code, new output since the offset |
| `nyx__machine_job_cancel` | Cancel a job (kills its whole process group) |
| `nyx__machine_list_files` | `{machine, path, depth?, glob?}` bounded listing |
| `nyx__machine_read_file` | `{machine, path, offset?, limit?, encoding? text\|base64}` paginated; binary files as base64 only when asked |
| `nyx__machine_write_file` | `{machine, path, content, encoding?, mode create\|overwrite\|append, expected_sha256?}` returns new sha256 |
| `nyx__machine_edit_file` | `{machine, path, old_string, new_string, replace_all?, expected_sha256?}`: exact-match replace (fails on 0 or ambiguous matches) |
| `nyx__machine_save_attachment` | Write one of this conversation's attachments (e.g. a generated image) to a path on the machine |
| `nyx__machine_share_file` | Attach an image file from the machine to the conversation, so the owner sees it (same rules as tool images: PNG/JPEG/GIF/WebP, ≤ 5 MiB, verified magic bytes) |
| `nyx__machine_computer` | Call one cua driver tool the machine advertises: `{machine, tool, arguments}` |

- **Output bounds.** NyxAgent hands each result to its model as one string
  truncated at 10,000 characters (`docs/chat/08-nyxagent-engine.md`, Tool
  images). So results are compact JSON, streams are capped (head and tail kept,
  with `truncated` flags and byte counts), and reads, job output and listings
  paginate. Hard caps are server-side and node-side.
- **Images.** Screenshots and other images from `nyx__machine_computer` follow
  the existing tool-image pipeline: chat keys get a text note plus an owner-only
  conversation attachment; pixels never enter the result text. The driver's
  accessibility and element state (`get_window_state` and so on) is text, and
  that is what the model works from.
- **Where tools appear.** Tool descriptions say machines are the owner's and
  that commands run with the node user's full permissions on that machine.
  NyxBot's and specialists' instructions mention machines only when the agent
  has one.

### D5. Protocol

- **Messages.** New node WebSocket messages for machine operations: a request
  from NyxID, a final result, and streamed output for jobs. They reuse
  request/response routing, cross-replica dispatch (`node_dispatch`) and the
  timeout patterns of `ssh_exec`/`ssh_exec_result` and proxy requests. Read
  `docs/NODE_PROXY_PROTOCOL.md` and follow its conventions; document the new
  messages there.
- **Signing.** Every machine request NyxID sends is signed with the node's
  signing secret, bound to request ID, node ID, operation and a digest of the
  parameters, with a timestamp and nonce the node checks (replay window). The
  node rejects unsigned, stale or replayed requests, even when
  `NODE_HMAC_SIGNING_ENABLED` is off for proxying.
- **Negotiation.** Nodes and servers that predate this ignore it. The server
  only sends machine requests to nodes that advertised machine capabilities.
  The node only accepts them when locally enabled.

### D6. On the machine (node agent)

- **Process model.**
  - Commands run as the node's OS user, never elevated, through the platform
    shell (`sh -lc` on Unix, PowerShell on Windows if the node supports Windows),
    with no TTY and stdin closed unless given.
  - Each command gets its own process group. A timeout or cancel kills the
    group (SIGTERM, then SIGKILL after a grace period).
  - Background jobs are bounded (max concurrent, retained 1 hour, output ring
    buffer), and survive a WebSocket reconnect but not a daemon restart.
- **Environment hygiene.**
  - Child processes get a clean environment: an allowlist (PATH, HOME, USER,
    LANG/LC_*, TERM=dumb, SHELL, TMPDIR) plus request-provided `env`, bounded.
  - They never inherit NyxID or node secrets: tokens, signing secret, keychain
    or credential-store paths, `NYXID_*` variables.
  - File tools refuse the node's own config/credential directories even inside
    a root.
- **Roots.**
  - File tools and `cwd` must resolve inside a configured root: canonicalized,
    symlinks resolved, no `..` escape, TOCTOU-safe opens (`O_NOFOLLOW` or
    re-checks) where the platform allows.
  - Writes are atomic (temp file plus rename) and keep existing permissions.
  - `expected_sha256` gives optimistic concurrency.
  - The shell itself can reach anything the node user can. That is what `shell`
    means, and `status`, docs and the UI say so plainly.
- **Git and other connected services** go through the machine's service
  gateway (D14). No credential is ever sent to the machine.
- **Transfers.** `save_attachment` streams the attachment from NyxID
  (owner-only, same conversation, size-capped). `share_file` streams a verified
  image into `assistant_attachments` (≤ 8 per turn, existing rules).

### D7. Computer use through the cua driver

- **What it is.** The cua driver (MIT, Rust, `libs/cua-driver` in trycua/cua)
  exposes GUI tools over MCP stdio (`cua-driver mcp`): screenshots/desktop
  state, window state with snapshot-bound elements, click, type, keys, scroll,
  drag, apps, windows, clipboard and so on (28 tools in the pinned 0.30.4 contract).
  It has no shell tools; ours cover that.
- **Managed install.**
  - `nyxid node machine enable --computer` installs a pinned cua driver release
    into the node's own directory. The download must be verified against
    SHA-256 values pinned in NyxID for each supported platform; there's no
    trust-on-first-use.
  - Alternatively, `--cua-driver PATH` uses an existing binary; its version is
    checked.
  - Never install or enable the optional `cua-perception` extension (AGPL).
  - Turn off cua telemetry.
- **Running.**
  - The node starts `cua-driver mcp` lazily on the first computer call, keeps
    one session, restarts it if it dies (bounded), and relays `tools/call`.
  - The advertised tool list comes from `tools/list`, filtered to the driver's
    public contract.
  - macOS: guide the owner through Screen Recording and Accessibility grants
    (`status` shows them). Linux needs a display; the container image provides
    one.
- **Permission mode.**
  - `computer_mode` selects cua's permission mode at launch (never over the
    tool protocol).
  - `unrestricted` needs the explicit flag, prints cua's own warning, and is
    the default only in the machine container image (D8), which is disposable.
  - In `standard` mode, actions cua reserves for human consent are refused on
    an unattended machine; the result says so.

### D8. Easy install on a host, a remote VM or a container

- **Host or VM:** the existing installer plus one setup command. The one
  command registers, enables the chosen capabilities, installs computer use if
  asked, and installs and starts the daemon (launchd/systemd, `--profile`
  aware):

  ```
  nyxid node setup --token nyx_nreg_… [--machine] [--computer] [--root DIR]
  ```

  `nyxid node register` and `daemon install` keep working as today.
- **Container:** a new image, `nyxid-node-machine`, published next to the
  existing node image by the Publish Images workflow. Remember that Dockerfiles
  stage workspace members by hand. It contains:
  - Ubuntu LTS, a non-root `agent` user, and a `/workspace` volume;
  - Xvfb with a light window manager;
  - the pinned cua driver, `computer_mode = unrestricted`;
  - git, curl, ca-certificates, python3, nodejs, ripgrep and build tools;
  - the node agent.

  On first start it registers with `NYXID_NODE_TOKEN` and persists its identity
  in a volume. `nyxid node docker … --machine` uses it.
- **Web (Assistant → Machines page):**
  - An "Add a machine" flow with tabs for this computer, a remote VM and Docker.
    It mints a registration token through the existing register-token API and
    shows the exact commands.
  - A plain safety note: use a VM or container, not your personal computer.
    Agents act with that user's full access, and prompt injection is possible.
    On a non-separated machine, commands can read the node token, signing
    secret, config and locally stored credentials. Setup, Assistant → Machines and machine
    status say so explicitly, with a persistent Not isolated badge when shell
    is enabled. Recommend the container or `--separate-users`; proceeding
    remains the owner's choice.
  - Machine settings open in a sheet with capabilities, roots, computer mode,
    `machine_confirm`, the single-user login opt-in and specialist grants.
    Studio Nodes retains generic infrastructure management and a read-only
    machine summary linking here.
- **NyxBot:** leads the whole setup from chat (D13).

### D8a. Out of scope for this release

Interactive TTY sessions and Windows containers. Write these down as follow-ups;
do not half-build them.

### D13. NyxBot sets machines up: it must be a breeze

The user: "nyxbot need to help user to set up this, so setting up should be a
breeze". The owner says "set up a machine for coding" (in the app or in a chat
app) and NyxBot takes it from there. Setup never exposes a credential to the
model: a registration token in the model's context would let a prompt injection
register an attacker's machine as the owner's, and agents would then send it
commands and files. So registration tokens appear only on NyxID pages the
owner opens, or on the machine itself, never in tool results or the transcript.

**Two ways in, both driven by NyxBot:**

1. **Setup link.** NyxBot calls `nyxid__machine_setup_link`:
   - Arguments: `{name?, where: this_computer | vm | docker, capabilities?, grant_to?: specialist name or id}`.
   - It returns a link to a one-page setup at `/assistant/machines/new?...` with the
     choices prefilled, like `nyxid__channel_bot_setup_link`.
   - On that page, the owner:
     - reviews the choices, including "let <specialist> use it" when `grant_to`
       is given;
     - reads the safety note;
     - gets a single copyable command with a fresh single-use setup token:
       - host or VM: one line that installs the CLI if missing and runs
         `nyxid node setup` with the token and the chosen capabilities;
       - Docker: one `docker run` with the token in an env var.
   - The page shows live progress: waiting for the machine, connected, and each
     enabled capability. When done it links back to the chat.
2. **Pairing code**, for a machine the owner is already logged in to, e.g.
   over SSH:
   - The owner runs `nyxid node setup --machine [--computer]` with no token.
     The machine prints a short code and a link, `…/assistant/machines/pair?code=…`, as
     in the device-login flow (`docs/DEVICE_LOGIN_PROTOCOL.md`).
   - To approve, the owner opens the link (the page shows the machine's
     hostname, OS, IP and requested capabilities, and requires an explicit
     confirm), or tells NyxBot the code.
   - If told the code, NyxBot calls `nyxid__machine_pair { code }`. That raises
     an action card showing the same machine details, which only the owner can
     decide (code-quoted confirmation in chat apps, as for other cards).
   - The code alone grants nothing. Until approved, the machine holds no
     credential and receives no requests.
   - The container image uses this when started without a token, printing the
     code and link in its logs.

**After connection:**
- **Watch.** Both paths are watched (`nyxbot_watches`, as for channel-bot setup
  links). When the machine registers and reports its capabilities, the waiting
  NyxBot thread is woken with a "machine connected" event.
- **Follow-up.** NyxBot can then confirm with `nyx__machine_list`, run a
  harmless check (e.g. `uname -a` / `git --version`), grant the machine to the
  specialist the owner named (the setup page's choice, or a normal grant), and
  continue the task.
- **Guidance.** NyxBot's instructions describe this flow, recommend a VM or
  container over a personal computer, and explain in one sentence what each
  capability allows. For computer use it tells the owner about the macOS Screen
  Recording/Accessibility prompts.
- **Failures.** They are reported back to the thread:
  - the setup expired;
  - the pairing was declined;
  - cua permissions are missing;
  - the machine is offline.

**Mechanics:**
- **Setup tokens.** Setup-link tokens reuse the node registration token
  machinery. They are single-use and short-lived, and carry the capabilities
  and grant intent. Only hashes are stored.
- **Pairing codes.** Pairing codes follow the device-code patterns already in
  the codebase: HMAC-stored codes, rate limits, expiry, and approve/deny racing
  atomically.
- **Result.** Both paths end in the same node registration as today, plus
  machine enablement on the node and the optional specialist grant.

### D14. Connected services from the machine: git, SDKs, scripts, CLIs

The user: "the git command and stuff can also be from the github service they
have connected on nyxid, and other services". Commands agents run on the
machine can use the owner's connected NyxID services (GitHub for `git`, OpenAI
for an image-generation script, any other connected service through its API)
**without the machine ever holding a credential**. Credentials stay in NyxID,
calls are audited, billed and approval-checked exactly like the agent's MCP
calls, and revoking a connection works immediately.

**Gateway:**
- **Least privilege per job.** `nyx__machine_exec.services` explicitly declares
  the services this command needs. NyxID validates slugs/IDs against live key
  access and stores both ID and slug on `MachineJob`. Omitted/empty means none.
  Every gateway call must match that declaration and remain accessible to the
  key. An undeclared request says to declare its service on `nyx__machine_exec`.
  Machine confirmation cards and audit records list those services.
- **Where it listens.** The node runs a service gateway on loopback only
  (`127.0.0.1`, an OS-assigned port), for the commands and jobs it starts.
- **Per-command token.** Each command or job gets a fresh random gateway token
  in its environment. The token is local to the node and meaningless to NyxID.
  It maps to that job and expires when the job ends. Requests without a live
  token are refused.
- **Forwarding.** Requests to `$NYXID_GATEWAY_URL/s/{slug}/{path}` are forwarded
  over the node's WebSocket as a node-signed "machine service call" and
  streamed back. Streaming responses and large downloads are streamed, not
  buffered. `Content-Encoding` and `Content-Length` are forwarded together on
  both relay hops; compressed SDK and git responses keep their original bytes.
- **Server-side execution.** NyxID runs the call through the existing proxy
  pipeline (`execute_proxy`) with the identity and authority of the chat key
  whose machine operation started that job: NyxBot's services, or the
  specialist's grants. That covers service allowlists, approvals, billing,
  node routing of node-held credentials, the platform-key ACL and audit.
- **Server-side checks.** NyxID honours a service call only when it names a
  job that NyxID itself started on that node, for that conversation, and that
  is still running. A compromised or rogue machine therefore cannot call
  services for another conversation, another agent or after the job.
- **Guests.** Guest turns never start machine operations, so they can never
  reach the gateway.

**Environment for commands:**
- **Always set:** `NYXID_GATEWAY_URL` and `NYXID_GATEWAY_TOKEN`.
- **Also set for commonly used SDKs**, only for services declared for this job:
  - `OPENAI_BASE_URL` / `OPENAI_API_KEY`, pointing to the gateway;
  - the equivalents for Anthropic and other `llm-*` catalog services whose SDKs
    honour a base URL;
  - the API key variable is set to the gateway token, which is useless outside
    this job.
- **Catalog is authoritative.** The server derives SDK variables from
  `inference.wire_protocol`, and git rewrites from catalog `git_http.origin` and
  `git_http.username`. GitHub OAuth/PAT seeds declare `https://github.com` with
  `x-access-token`. The signed exec request carries the environment spec; node
  releases contain no service-slug mappings. Git requires a non-platform
  connected credential. Discovery reuses the shared catalog/MCP ACL resolver,
  and execution reuses the middleware's API-key authority constructor.
- **Visibility.** `nyx__machine_list` shows which connected services are
  reachable from the machine and which environment variables are set.

**Git over the gateway:**
- **Remote side.** NyxID gains a git smart-HTTP route for connected git hosts:
  GitHub is required, and GitLab/Bitbucket follow if they are in the catalog.
  It forwards `info/refs`, `git-upload-pack` and `git-receive-pack` to the
  host (e.g. `https://github.com/{owner}/{repo}.git/...`) with the owner's
  connected credential injected server-side as the host expects (GitHub:
  Basic `x-access-token:<token>`). The same credential resolution, ACLs and
  audit apply, and platform keys are never used. Request and response bodies
  stream; the route must handle multi-GB clones and pushes within the existing
  proxy body limits, raising them for this route only if needed and
  documenting it.
- **Machine side.**
  - For each command, the node points git at the gateway through per-process
    configuration only: `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/`GIT_CONFIG_VALUE_n`
    for `url.<gateway>/git/github.com/.insteadOf https://github.com/` (and
    `git@github.com:`), plus the gateway token as an extra header for that
    URL.
  - Nothing is written to global or repo git config.
  - Remote URLs in cloned repos stay `https://github.com/...`.
  - So plain `git clone`, `fetch`, `pull` and `push` of private repos just work
    inside any command declaring the connected GitHub service in `services`.
  - With no git host declared, git goes direct, and public repos
    still clone.
- **Pushing.** A push goes through the approval pipeline like any other
  service write. With `machine_confirm = changes`, the command that pushes
  already needed a card.
- **The GitHub CLI and other API tools.** Use the REST API through the gateway
  (`curl -H "Authorization: Bearer $NYXID_GATEWAY_TOKEN"
  $NYXID_GATEWAY_URL/s/api-github/...`). Document this for agents. Do not
  hand CLIs a real token.

**Failures:** a service the agent may not use, one that isn't connected, or one
that needs approval. The gateway returns a clear HTTP error with the NyxID error
code and message, and the tool result shows it. NyxBot can then offer a
connect link or a permission request, as for MCP service calls.

### D15. Live desktop: watch the agent, take over, hand back

The user: "best is if the node can help to stream the desktop or browser etc
back to nyxbot so it can be controlled or watched on nyxbot on the web like
grok bot, muse …, which with the cua it can pass to user if require like
logging to a website and then hand back the control to nyxbot".

**What the owner sees**
- In NyxBot on the web, a conversation whose agent uses a machine's `computer`
  capability shows a **live desktop panel**:
  - it streams the machine's screen in near real time;
  - it shows the agent's cursor and actions as they happen;
  - it can be expanded or popped out.
  The Assistant → Machines page can open the same view for any computer-capable machine.
- **Controls:**
  - **Take control** switches the controller to the owner. The owner's mouse,
    keyboard (including typing and shortcuts), scroll and clipboard paste in the
    panel drive the machine.
  - **Hand back** returns control to the agent, with an optional note (e.g.
    "logged in").
  - **Stop** stops the agent's turn (existing Stop).

**Agent asks for the owner (handoff)**
- **The request.** A new tool, `nyx__machine_request_control {machine, reason}`,
  lets the agent ask the owner to take over, for a login, a CAPTCHA, a payment
  confirmation or anything the agent should not do. It raises a visible request:
  - in the web conversation, a banner or card: "NyxBot needs you on <machine>:
    <reason>" with a **Take control** button;
  - in chat apps, a message with a link that opens the live panel (owner web
    session required);
  - a push notification where configured.
- **While waiting.** The agent's turn ends, and it does not burn a turn polling.
  When the owner hands back, the waiting thread is woken with an event turn
  carrying the owner's note, as in the existing watch/wake patterns. If the
  owner takes control on their own initiative, the agent is paused the same
  way and woken on hand-back.

**While the owner is in control**
- **The agent is fully locked out.** Every `nyx__machine_computer` call, and any
  other machine call on that machine that would observe or act on the desktop,
  returns `owner_in_control` with instructions to wait.
- **Privacy.** Nothing the owner types, and no frame captured while the owner
  is in control, is ever stored, attached, logged or given to the agent.
  Passwords typed during a login never reach the model. After hand-back, the
  agent observes the resulting state fresh.
- **Commands.** Shell and file operations on that machine are also refused
  while the owner has control, since they could observe the session. Other
  machines are unaffected.

**Streaming mechanics**
- **Node side.**
  - Agent actions and observations use cua MCP. The human live view uses
    in-process X11 capture on Linux and ScreenCaptureKit on macOS (the same
    Screen Recording permission). Linux owner input uses a separate XTest
    connection; macOS uses a separate human cua session.
  - Capture runs at 30 Hz with changed 64-pixel tile detection, merging dirty
    tiles into a JPEG rectangle. Idle screens send no frames. Each rectangle
    identifies its base sequence; missed frames request a fresh complete frame.
    Cap dimensions at 1920×1200 and traffic at 2 MiB/s. At 1280×800, acceptance
    is ≥15 fps during activity (aim 24–30), p95 owner input-to-frame ≤100 ms.
  - No controller/session lock is held across capture, encoding or input I/O.
    Takeover flips a revisioned cancellation signal immediately, kills the
    in-flight agent cua session and command groups, cancels file workers, and
    discards late results. Target ≤150 ms even with a slow action/file transfer.
- **Transport.**
  - Frames and input events travel as binary WebSocket frames tagged by a
    desktop-session ID, over the node's existing WebSocket to NyxID.
  - NyxID relays them to the owner's browser over an authenticated WebSocket
    under `/assistant/nyxagent/...` (human-only, owner-only; add it to
    `delegated_read_denied_path` if it is a GET upgrade).
  - Cross-replica: follow the existing browser SSH terminal (`/ssh/{id}/terminal`)
    and node dispatch patterns, so the browser and the node can be on different
    replicas.
- **Sessions.**
  - Desktop sessions start on demand (the panel opens, or the agent uses
    computer tools) and end when no viewer and no agent activity remain, after
    a short idle timeout.
  - Several owner tabs may watch; only one controller at a time.
  - Bandwidth and frame caps apply per session.
- **Browser.** The machine container image includes Chromium, run as the
  node-managed browser of D16 with a persistent profile in the machine volume.
  Chromium's renderer sandbox is enabled through user namespaces and seccomp;
  the container uses NyxID's narrowly extended Docker seccomp profile with
  `--security-opt seccomp=...`, no added capabilities and no `--no-sandbox`.
  All dropped children set `PR_SET_NO_NEW_PRIVS` before exec.
  Agents can browse, and the owner can take over web logins in it. Sign-ins
  persist across tasks, so the owner logs in once, not every time.
- **Audit.** Metadata only: session start and end, control changes and the
  reason given by the agent. No frames, keystrokes or clipboard contents.

**Tests**
- Controller state machine: agent → owner → agent, the agent locked out while
  the owner controls, wake on hand-back, owner-initiated takeover.
- Frames during owner control are never persisted or returned to agents.
- Auth: only the owner, never guests or other users.
- Cross-replica relay.
- Frame diffing and rate limits.
- Frontend panel: watch, take control, hand back, request banner.
- An end-to-end run in the machine container: stream, owner types into a
  browser field, hand back, agent continues.

### D16. Saved logins: agents sign in without ever seeing the password

The user chose to add this in this release, after comparing OpenAI's dots
("Dots sign in with saved passwords without exposing them to the model"). The
agent signs in to websites on the machine using logins the owner saved in
NyxID. No password, one-time-code secret or username value ever enters the
model's context, a tool result, a transcript, a log, an audit record or the
machine's disk. Owner takeover (D15) stays available for everything else.

**Saved logins (NyxID)**
- **Model.** A new collection of saved website logins with:
  - an owner (a person, or an org, managed by org admins through
    `resolve_owner_access`);
  - a label;
  - allowed origins (exact `https://` origins, e.g. `https://github.com`,
    `https://accounts.google.com`);
  - the username, password and optional TOTP secret (entered raw or as an
    `otpauth://` URI), all envelope-encrypted with `EncryptionKeys`;
  - timestamps and last use.
- **Management.**
  - Owner CRUD on a new **Saved logins** page, through human-only routes.
  - Secrets are write-only: the API never returns them, only the label,
    origins, a masked username hint and whether a password and TOTP are set.
  - Replacement is atomic.
  - Deleting the owner purges the owner's saved logins.
  - Secret-bearing structs have redacted `Debug` and use `Zeroizing`.
- **Who may use a login.** NyxBot may use all the owner's saved logins.
  Specialists may use only the logins granted to them:
  `AssistantAgent.saved_login_ids`, stored beside `grants`. The grant tools and
  the Grants form accept logins, and an ungranted login raises the usual
  permission request. Guests never can.
- **NyxBot's role.** It sees labels and origins (in `nyx__machine_list` or a
  small `nyx__saved_logins` listing), never values. It sends the owner to the
  Saved logins page through `nyxid__settings_link` (new area `saved_logins`),
  and never asks for passwords in chat.

**Signing in (machine)**
- **The tool.** `nyx__machine_fill_login {machine, login, field: username |
  password | one_time_code}` fills the currently focused field of the node's
  **managed browser** with that value. The agent navigates and focuses fields
  with the computer tools as usual, then submits the form with a click or key.
  The result is only `{filled: <field>, login: <label>, origin: <matched
  origin>}`.
- **The managed browser.**
  - The node launches Chromium itself, with a persistent profile in the machine
    volume. Password saving and autofill are disabled, and there is no
    DevTools access of any kind: `DeveloperToolsAvailability` is disallowed by
    managed policy, which also disables DevTools pipes. The live desktop (D15)
    and the computer tools operate it like any window.
  - Filling goes through a **NyxID filler extension**:
    - It is force-installed by managed policy
      (`ExtensionInstallForcelist`/`ExtensionSettings` pointing to a local,
      signed, supervisor-owned package, with a pinned extension ID). The user
      and the agent cannot disable or remove it.
    - It talks only to a **native messaging host**, registered in a
      supervisor-owned manifest, which talks only to the node supervisor over
      a socket that the agent user cannot open.
    - The policy files, the extension package and the host manifest are
      writable only by the supervisor (root in the container, or the setup
      flag's service user on a separated VM).
- **Checks before typing (in the extension's isolated world).** The extension
  verifies that:
  - the tab's top-level origin (or the focused frame's origin) is one of the
    login's allowed origins;
  - the focused element is a suitable input: `password` for passwords;
    `text`, `email` or `tel` for usernames; text or numeric for codes.

  It then inserts the value with the input events a real keystroke produces.
  Any mismatch, including a different site, a different field or no managed
  browser, refuses without typing and says why. After a password, it pins that
  field to `type="password"` (a mutation observer in the isolated world) until
  submit or navigation.
- **Protocol.** Each fill is one supervisor→extension request (nonce, allowed
  origins, field kind, value) and one result (filled or refused, with a
  reason).
  - The value never touches the page's own JavaScript world, the DOM before
    insertion, or disk.
  - The request and native-messaging paths carry no value in logs.
- **Other platforms.** On macOS and single-user hosts, the same policies and
  extension are installed through the platform's managed-policy location,
  which needs admin once at setup; the setup asks for it. If the owner
  declines, saved-login filling there stays unavailable and the setup says
  why. Implement and test the Linux container path end to end, and implement
  the macOS policy path with tests for the generated policy.
- **One-time codes.** NyxID computes the current TOTP code (RFC 6238) and
  sends only the code, never the TOTP secret.
- **Transport.** The value travels only inside a signed machine request
  (D5). The node holds it in memory only until it is typed, and does not log
  it.
- **Output scrubbing.** For a period after typing (at least the desktop
  session), every output the node returns from that machine has the typed
  values replaced with `[redacted]`. That covers computer-tool text,
  accessibility trees, command output, file reads and job output. It is
  defence in depth against a "show password" toggle or a page echoing the
  value.
- **Isolation: recommended, the owner decides.** We cannot control what the
  owner installs the node on, so NyxID warns and recommends; it does not
  forbid.
  - **Isolated machines.** In the machine container, the node supervisor runs:
    - the browser and the cua driver as a `browser` OS user;
    - agent commands and file tools as an `agent` user (no ptrace, no access to
      the browser's processes or `0700` profile directory);
    - the node itself as a supervisor that drops privileges per child.

    A setup flag creates the same user separation on a VM; implement and
    document it. Such machines report `isolated` and saved logins work there
    without further steps.
  - **Single-user machines** (the owner's laptop, a plain VM). Saved-login
    typing is **off until the owner allows it for that machine**, through a
    per-machine setting on the Assistant → Machines page (owner-only, human-only route; NyxBot
    cannot change it). Turning it on shows a plain warning: agent commands on
    this machine run as the same user as the browser, so a misbehaving or
    prompt-injected agent could read what is typed into it. It also recommends
    the container or a separated VM.
  - **After the owner allows it,** typing works as on an isolated machine; the
    origin and field checks, TOTP handling and output scrubbing all still
    apply.
  - **Until then,** `nyx__machine_fill_login` returns a clear result saying the
    machine is not isolated and how the owner can allow it or use an isolated
    machine. NyxBot relays that with the Assistant → Machines settings link. The setup page
    (D13) and `nyxid node machine status` show the same warning and
    recommendation.
- **What is guaranteed, precisely (trust model).** The approved website and
  the node-managed browser profile are **trusted recipients** of a typed
  secret, as when a person signs in. The website may keep or re-display what it
  receives, and NyxID cannot control that.
  - **What NyxID guarantees:** it never places a secret in any channel it
    controls: model context, tool results, transcripts, logs, audit, the
    machine's disk or environment, the command line, or any TCP debugging
    port. It types only into approved origins and suitable fields.
  - **Closing the realistic ways back out.** In the managed browser, the agent
    cannot pull the value back out through it:
    - DevTools and `javascript:` URLs are disabled by Chromium policy
      (`DeveloperToolsAvailability` = disallowed, `URLBlocklist` for
      `javascript:*`), in a policy file only the node supervisor can write.
      Filling uses the force-installed filler extension, not DevTools;
    - the browser's password manager and autofill are off;
    - after typing a password, the filler extension pins that field to
      `type="password"` in its isolated world, so a "show password" toggle
      cannot reveal it until the form is submitted or the page navigates;
    - browsers already refuse to copy or cut from password fields.
  - **Defence in depth.** Output scrubbing covers the exact value and its
    common encodings (base64, base64url, hex, URL-encoding), from all outputs
    of that machine, for the scrub period.
  - **The one stated residual:** a website that deliberately writes the
    password back into its page as ordinary text could make it visible on
    screen, like to a person sitting at the keyboard. The Saved logins page,
    the tool description and the docs say this in one sentence, next to
    recommending owner takeover (D15) for the most sensitive accounts.
  - **Tests** prove each measure:
    - DevTools and `javascript:` blocked;
    - the extension is force-installed and cannot be disabled;
    - the native-messaging socket is unreachable for the agent user;
    - reveal-toggle pinning;
    - copy refused;
    - encodings scrubbed;
    - the no-secret-in-any-NyxID-channel sweep.
- **Owner confirmation.** A per-login option makes each sign-in raise an action
  card first (default off). A machine's `machine_confirm` applies as for other
  changing operations.
- **Audit.** Metadata only: login ID, machine, field kind, matched origin and
  outcome; never values.

**Errors:** add codes in the machine block (D10) for:
- login not found or not usable;
- origin mismatch;
- wrong field;
- managed browser unavailable;
- machine not isolated.

**Tests**
- Encryption at rest, and that the write-only API never returns secrets.
- Authorization: NyxBot, granted and ungranted specialist, guest, org.
- Origin and field checks in the filler extension (unit tests plus the real container browser), and refusal on a mismatch.
- TOTP code correctness, and that the secret is never sent.
- Output scrubbing.
- Single-user machines: off until the owner allows it (with the warning), then working.
- In the container end to end: sign in to a test site with username, password
  and TOTP.
- Across the whole transcript, tool results, logs and audit, a test asserts no
  secret value appears.
- Frontend: the Saved logins page and the grants picker.

### D17. Performance (measured, not assumed)

The user: "nyxbot … will be a competitor to muse/grok bot and dots … we need
to ensure the performance as well". This PR must show, with numbers, that
machine access is fast and does not slow anything else down:

- **No hot-path regressions.** Proxy, MCP `tools/list`/`tools/call` for
  callers without machines, turn start and channel inbound gain no extra
  database round trips. Machine lookups for chat keys are batched once per
  request, with no N+1 over nodes, agents or logins.
- **Machine operations.**
  - Measure the NyxID-side overhead of `nyx__machine_exec` (tool call to node
    and back, excluding the command's own runtime) on a local node: at most
    50 ms p95 added on loopback.
  - File reads and writes stream without whole-body copies beyond one bounded
    buffer.
  - Background job output is ring-buffered, never unbounded in memory.
- **Gateway (D14).**
  - Throughput of a large download through the gateway (≥ 100 MB) and a git
    clone of a medium repository through the smart-HTTP route, compared with
    direct. Report both.
  - Streaming must be end to end, with no full-body buffering.
  - The job-binding check is one indexed lookup, or cached per job.
- **Live desktop (D15).**
  - Report the frame rate and bandwidth achieved in the machine container
    (1280×800) and on macOS for idle, typing and scrolling.
  - Changed-frame diffing must keep an idle screen near zero bandwidth.
  - Input-to-screen latency for the owner in control: measure and report.
- **Setup (D13).** From the owner running the command to NyxBot being woken,
  under 10 s once the machine connects (watch plus change stream, not only the
  15 s sweep).
- **Benchmarks.** Add repeatable timing tests or benchmarks for the hot paths
  above. Report the numbers in the final report and in this document.

Measurements and reproduction commands are under
[Validation and measurements](#validation-and-measurements).

### D9. Audit, logs and privacy

- **Audit.** Every machine operation writes a metadata-only audit event
  (`machine_operation`) through the audit service: node ID, operation, tool,
  conversation, agent role, outcome, exit code, duration, byte counts and
  whether a card was used.
- **Never logged or audited:** command text, arguments, environment values,
  file paths or contents, stdout/stderr, git URLs with credentials,
  credentials, screenshots or clipboard. The same applies to the node daemon's
  logs.
- **Redaction.** Saved-login and signing types have redacted `Debug`, and
  decrypted secret material is wrapped in `Zeroizing`. Connected-service
  credentials stay in the server proxy pipeline; there is no git credential
  payload sent to the node.

### D10. Errors

- **New codes.** Reserve a new numeric block in `backend/src/errors/mod.rs`
  (12400–12413, machine nodes, including D15/D16) for:
  - machine access not enabled for the capability;
  - a machine the caller may not use;
  - a path outside roots;
  - a job not found;
  - a confirmation pending or declined;
  - an unavailable cua driver;
  - an output or transfer limit;
  - owner control, single-user saved-login opt-in, login access, origin/field
    mismatches and managed-browser readiness.
- **Existing codes.** Reuse the node codes (8000/8001/8002) for not found,
  offline and timeout.
- **Documentation.** Add the block to CLAUDE.md's reserved table.
- **Tool results.** Tool results carry the stable code and a short
  instruction.

### D11. Compatibility and rollout

- **Additive.** Every model field is additive and serde-defaulted. No new
  environment variables unless unavoidable; if one is added, document it in
  `docs/ENV.md` and CLAUDE.md.
- **Mixed versions.** Old nodes are unchanged and never receive machine
  requests. Old servers never send them. Old frontends ignore the new fields.
- **Existing features.** Proxy, credential, SSH and oracle behaviour are
  unchanged.

### D12. Documentation

This document describes what shipped. Also update:
- `docs/NYXID_NODE.md` (setup, machine commands, container);
- `docs/NODE_PROXY_PROTOCOL.md` (messages, signing);
- `docs/chat/09-nyxbot-orchestrator.md` (machines for NyxBot and specialists,
  grants, permission requests, guests never);
- CLAUDE.md: the Node Proxy Conventions and NyxAgent paragraphs, error codes,
  and commands (a short CLAUDE.md paragraph pointing here, not a copy).

## Acceptance

- **NyxBot-led setup.** In the app and in a linked chat app, "set up a machine
  for coding" leads to the setup link or pairing code; the machine connects;
  NyxBot is woken, verifies it and continues. No registration token or other
  credential appears in any tool result or transcript (asserted by tests).
- **Owner flow.** With a node set up by `nyxid node setup --machine`, NyxBot, on
  an owner turn in the app and in a linked Telegram chat, can:
  - list the machine;
  - run commands (foreground and background, with cancel);
  - read, write and edit files;
  - run `git clone`/`pull`/`push` of a private GitHub repository inside an
    ordinary command, using the owner's connected GitHub service through the
    gateway; the credential never reaches the machine, as asserted by tests on
    the node side (environment, git config, remote URL, process arguments);
  - run a script that calls another connected service (e.g. OpenAI image
    generation) through the gateway and writes the result on the machine; and
    save a conversation attachment to the machine;
  - share a screenshot back to the conversation.
- **Specialists.** A specialist can do the same only on a granted machine; an
  ungranted one raises a permission request to NyxBot. Guests can do none of
  it.
- **Computer use.** It works in the machine container end to end (cua driver
  under Xvfb): desktop state, click and type.
- **Live desktop (D15).** The owner watches the agent work live in NyxBot on
  the web, takes control for a login (the agent requests it with
  `nyx__machine_request_control`), and hands back; the agent continues without
  ever seeing what the owner typed.
- **Saved logins (D16).** With a saved GitHub login (password plus TOTP),
  NyxBot signs in to github.com in the machine container's managed browser by
  itself. No secret value appears anywhere the model, transcript, logs or audit
  can see.
  - Typing is refused on a different origin or a different field.
  - On a single-user machine it works once the owner allows it for that
    machine, after the warning; until then the result explains how.
- **Confirmation.** `machine_confirm = changes` gates exactly the changing
  operations with cards.
- **Tests.** Every D-item has tests:
  - authorization matrix: owner, NyxBot, granted and ungranted specialist,
    guest, other user, org admin and non-admin, offline node, capability not
    advertised, node-local disabled;
  - signing and replay rejection;
  - environment scrubbing;
  - root confinement including symlink escapes;
  - timeout process-group kill;
  - output caps and pagination;
  - job lifecycle across reconnect;
  - the gateway: a live job token, the server honouring only jobs it started
    for that conversation and node, specialist scope, expiry at job end,
    streaming, and git smart-HTTP clone and push against a test git server;
  - cua relay against a fake MCP stdio server, plus pinned-checksum
    verification;
  - confirmation cards;
  - attachments;
  - frontend (Add a machine flow, node details, grants picker);
  - container image build (CI).
- **Existing tests.** The backend, CLI and frontend suites stay green. Run
  `cargo fmt`, `clippy -D warnings` (CI uses Rust 1.98.1), the frontend lint,
  tests and build, and `npm run build:wizard` if wizard sources change.

- **Performance (D17).** The measurements are reported and meet the stated
  budgets, or document why not and the plan to meet them.

## Integration with NyxBot automations and service pools

Machine tools in scheduled turns retain the same live machine/login grants,
owner-control lockout and `machine_confirm` rules as owner-initiated turns.
Webhook turns add their per-run confirmation policy. One exact-argument,
one-use owner action card satisfies both policies when both require approval.
Exec, file writes/attachment saves, job cancellation and changing computer
input are destructive because they can overwrite data or interrupt arbitrary
work. Saved-login filling and requesting owner control are changing, but not
destructive: they are restricted field insertion or an owner handoff rather
than arbitrary execution. Machine listing, saved-login metadata, file listing,
file reads and job status pass the webhook read-only gate; machine-level `all`
confirmation still applies to operations as configured. Both direct native
calls and `nyx__call_tool` use these checks. Guests and developer OAuth tokens
cannot use machine tools or the human desktop/control routes.

Machine gateway declarations bind service or pool IDs and slugs. A declared
pool accepts bounded, buffered JSON/form requests with its normal priority or
AI routing, per-member authorization and failover. Pool access confers no
member grant: only members allowed by the chat key and the owner's live ACL
can run. AI chat pools generate OpenAI SDK variables; same-API pools use their
catalog protocol. Opaque streamed uploads, including git, require a concrete
connection and cannot enter a pool or retry on another credential node.
Uploads use the shared HTTP cancellation path through response completion.

## Assistant workspace

The assistant sidebar's Workspace group is Home, Automations, Machines,
Plugins, Artifacts (coming soon), Approvals and Activity (coming soon), with
the same active state in the mobile drawer. Both NyxAgent and legacy assistant
engines use these workspace views inside `AssistantShell`.

- `/assistant/automations` manages schedules and assistant webhooks; `setup`
  and `agent` preserve setup watches and agent filters. The shipped
  `/automations` URL redirects here with both parameters intact.
- `/assistant/machines` lists usable personal and organization machines, their
  connection and capability state, desktop access and settings. Its Saved
  logins tab is `/assistant/machines?tab=logins`.
- `/assistant/machines/new` and `/assistant/machines/pair` keep setup and
  pairing inside the assistant; `/assistant/machines/{id}/desktop` opens a
  standalone full-screen desktop.

Studio retains Nodes, generic node details and Developer → Triggers for webhook
secrets/replay. Machine settings are edited only in the assistant. Server tools,
setup APIs and notifications use `services::assistant_links::AssistantPage`
for browser links; API paths do not change. `settings_link` areas `automations`,
`machines` and `saved_logins` open these views, while `triggers` still opens the
developer page. Webhook prefill accepts both `automations` and the legacy
`triggers` area. All new pages use the assistant's shared authentication guard
and validated search parameters; backend human-only checks remain authoritative.

## Validation and measurements

| Acceptance criterion | Automated coverage |
|---|---|
| NyxBot-led setup and no credential in chat | `machine_setup_tools_and_owner_cards_never_contain_registration_credentials`, pairing approve/deny races, atomic grant application, durable watch dedupe, actual change-stream wake benchmark; setup page tests cover review and live progress. |
| Owner commands, jobs, files, git, services and attachments | Production CLI through real loopback WS; command cancellation/output tests, anchored file operations, job-token reconnect/expiry tests, real smart-HTTP clone/fetch/pull/push plus service proxy fixture; screenshot attachment ownership/magic/turn limits and container file transfers. |
| Specialist grant and guest exclusion | `machine_specialist_permission_is_explicit_durable_and_revocable`, `machine_authority_owner_guest_org_membership_offline_and_capabilities`, MCP discovery/call authority matrix and live specialist gateway scope. |
| Computer use | Production-image test uses the pinned driver under Xvfb for observation, clicks, typing and screenshots. |
| Owner takeover and hand-back | Durable controller race/recovery tests, exact revision fencing, owner takeover shell/file/computer lockout and wake note, browser WS authorization, cross-replica relay; frontend panel and real container input/hand-back tests. |
| Saved login sign-in and privacy | Actual HTTPS username/password/TOTP sign-in in container Chromium; force-installed extension, policy denial, origin/field mismatch, password pinning, copy refusal, OS/socket isolation, encoded-output scrubbing and secret sweeps; backend storage/grant/org/card/opt-in tests and Saved logins form tests. |
| Confirmation | `machine_confirmation_is_bound_to_parameters_and_consumed_once`, observation-versus-change classification and saved-login per-use confirmation. |
| Compatibility and all decisions | CLI/backend/frontend regression suites cover additive models, legacy capabilities and D1–D17 authority boundaries. |
| Namespace isolation and release artifacts | Agent children inherit NoNewPrivs and a namespace-denying seccomp filter; container tests assert syscall errors, git operation and renderer sandboxing. Frontend build tests enforce the canonical profile and server-version image tag. |
| Declared pools | Buffered same-API/AI failover tests, declaration and SDK-variable tests, live member-scope denial, and streamed-body refusal before polling or dispatch. |
| Assistant workspace | Production route-tree tests cover both engines, shell/mobile navigation and active state, parameter-preserving automation redirect and standalone desktop; machine settings/grants and shared server link outputs are tested. |
| Performance | Indexed lookup/query-count tests, actual CLI loopback timing, 100 MiB gateway/git comparison, desktop scenarios and change-stream wake measurement. |

Measured on an Apple M2 / 16 GiB host, with the production Linux arm64 machine
image in Docker Desktop at 1280×800. The desktop run passed during concurrent
compilation on the shared host.

| Backend measurement | Direct / budget | Through NyxID |
|---|---:|---:|
| Exec overhead over loopback WS, command runtime excluded | ≤ 50 ms p95 | **7.036 ms p50 / 36.879 ms p95** |
| In-process signed exec dispatch | 100 samples | **2.420 ms p50 / 2.590 ms p95** |
| 100 MiB download | 0.197 s / 508.42 MiB/s | **0.193 s / 518.13 MiB/s** |
| 12 MiB git clone | 0.676 s | **0.768 s** |
| Capability report → durable NyxBot wake | < 10 s | **17.89 ms** |

Exec uses 100 samples after 10 warmups through the actual CLI runtime. Gateway
and git measurements use local HTTPS fixtures with streamed bodies; the same
test verifies clone/fetch/pull/push. Direct runs first. Setup timing uses the
real change stream and excludes download, installation and human approval.
Backend benchmarks use the debug test binary on a shared development host.
MongoDB 8 runs as a single-member replica set with a 4.5 GiB container memory
limit and a 0.25 GiB WiredTiger cache. Direct/gateway throughput differences
within a few milliseconds reflect local benchmark noise, not an acceleration
claim.

| Desktop scenario | Changed frames/s | Frame bytes/s |
|---|---:|---:|
| Idle | 0 | 0 |
| Typing | 30.304 | 196,569 |
| Scrolling | 29.329 | 2,026,690 |

Owner input-to-frame latency: **30.927 ms p50 / 70.089 ms p95**.
Takeover with a stalled cua action and a 5 MiB upload in flight: **0.790 ms**.
A 4 MiB file transfer round trip: **147.859 ms**. Native capture targets 30 Hz;
scrolling reverses every six actions to avoid an idle page boundary. JPEG dirty
rectangles avoid full-frame encoding for text changes and need no video decoder
startup; sequence checks recover dropped rectangles with a full frame.

The macOS driver reports Screen Recording and Accessibility permission both
missing on the validation host. Consequently macOS desktop fps, bandwidth and
input latency are **unmeasured**. Enable those OS permissions for the launching
app and rerun the documented desktop benchmark to close that validation gap;
macOS policy-generation and RAM-only capture-volume tests pass.

### Repeatable checks

Use Rust 1.98.1 and a replica-set MongoDB. Run backend chunks sequentially:

```sh
export CARGO_INCREMENTAL=0
export NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27024/?directConnection=true'
cargo +1.98.1 test -j 1 -p nyxid --bin nyxid-server -- handlers:: --test-threads 2 --skip curation_concurrent_writers_and_shared_budget
cargo +1.98.1 test -j 1 -p nyxid --bin nyxid-server -- services:: --test-threads 2 --skip curation_concurrent_writers_and_shared_budget
cargo +1.98.1 test -j 1 -p nyxid --bin nyxid-server -- --skip handlers:: --skip services:: --skip curation_concurrent_writers_and_shared_budget --test-threads 2
cargo +1.98.1 test -j 1 -p nyxid-cli -p nyxid-machine
cargo +1.98.1 fmt --all -- --check
rustup run 1.98.1 rustfmt --edition 2024 --check cli/src/node/machine/runtime.rs
git diff --check
cargo +1.98.1 clippy -j 1 --workspace --all-targets -- -D warnings
```

In `frontend/`, run `npm run lint`, `npm test`, `npx tsc -b`, and `npm run build`.
Run `node --test cli/tests/machine_filler.test.mjs` from the repository root.
Run each benchmark alone after builds/tests stop:

```sh
cargo +1.98.1 test -j 1 -p nyxid --bin nyxid-server -- machine_loopback_exec_performance --ignored --nocapture --test-threads 1
cargo +1.98.1 test -j 1 -p nyxid --bin nyxid-server -- machine_gateway_streaming_and_git_performance --ignored --nocapture --test-threads 1
cargo +1.98.1 test -j 1 -p nyxid --bin nyxid-server -- machine_setup_change_stream_wakes_thread_without_sweep --nocapture --test-threads 1
cargo +1.98.1 test -j 1 -p nyxid --bin nyxid-server -- machine_exec_dispatch_performance --ignored --nocapture --test-threads 1
docker build -f cli/Dockerfile.machine -t nyxid-node-machine:local .
docker build --build-arg MACHINE_IMAGE=nyxid-node-machine:local -f cli/tests/Dockerfile.machine -t nyxid-machine-e2e:local .
docker run --rm --shm-size=256m --security-opt seccomp=cli/resources/machine-container/seccomp.json nyxid-machine-e2e:local
```

The container test uses the production CLI, signed extension, native host and
pinned cua binary at 1280×800. Its output contains timings and frame byte counts;
it does not write screenshots. Idle means a settled page with an unfocused text
field. Typing and scrolling are continuous acknowledged input; scrolling
reverses every six actions to avoid measuring an idle page boundary. Alternating
scroll events then measure input-to-frame latency. Action counts accompany frame
counts.

The macOS benchmark requires a logged-in desktop, Google Chrome and Screen
Recording/Accessibility permission for the app launching the test:

```sh
NYXID_MACHINE_BENCH_CUA="$HOME/.nyxid-node/cua/cua-driver-rs-0.30.4-darwin-universal/cua-driver" \
  CARGO_INCREMENTAL=0 cargo +1.98.1 test -j 1 -p nyxid-cli --bin nyxid -- \
  macos_desktop_performance --ignored --nocapture --test-threads 1
```

For a named profile, set the driver path under `~/.nyxid-node/profiles/NAME/`
instead. Install computer support with `nyxid node machine enable --computer`
first if needed. The test opens an isolated 1280×800 Chrome window on the main
desktop, keeps captures in RAM, and prints the same scenario/fps/bytes/actions
table as the container plus input p50/p95. The streamed canvas follows the
screen size, capped at 1920×1200, so keep other windows idle during measurement.

### Rollout and container upgrades

Upgrade the server and web UI before enabling new machine capabilities; legacy
nodes remain credential/SSH nodes until explicitly enabled. Setup reads the
server's release from `/api/v1/public/config` and pins the Docker image to that
exact tag. The web build serves `cli/resources/machine-container/seccomp.json`
as `/machine-seccomp.json` and rejects a different published profile. No
separately maintained frontend copy exists.

To upgrade a machine container, finish or cancel its jobs, stop the container,
then recreate it with the new server-matching image tag and the same identity
and workspace volumes. Do not delete those volumes or mint a new setup token:
the persisted identity reconnects the existing node. Download the seccomp
profile from the upgraded web UI and retain `--security-opt seccomp=PATH`,
`--shm-size=1g`, `NYXID_NODE_URL`, the selected capabilities and the restart
policy. For a container created by the setup page as `my-machine`:

```sh
# Set these to the deployed server release and URLs.
nyx_machine_release=0.39.0
nyx_machine_web=https://nyxid.example
nyx_machine_ws=wss://nyxid.example/api/v1/nodes/ws
curl -fsSL "$nyx_machine_web/machine-seccomp.json" -o machine-seccomp.json
docker pull "ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:$nyx_machine_release"
docker stop my-machine
docker rm my-machine
docker run -d --name my-machine --restart unless-stopped --shm-size=1g \
  --security-opt seccomp=machine-seccomp.json \
  -v my-machine-identity:/var/lib/nyxid-machine \
  -v my-machine-workspace:/workspace \
  -e "NYXID_NODE_URL=$nyx_machine_ws" \
  "ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:$nyx_machine_release" \
  --shell --files --computer
```

On every Linux install, including separated VMs, command/file children apply
`PR_SET_NO_NEW_PRIVS` followed by their own seccomp filter: `unshare`, `setns`
and namespace-bearing `clone` return EPERM; `clone3` returns ENOSYS so libc can
fall back to ordinary `clone`. Browser and cua children retain the namespace
support required by Chromium's sandbox. This is defence in depth; use the
container or `--separate-users` for browser/agent OS-user isolation.

The pinned driver is `cua-driver-rs-v0.30.4`. Release archives were verified
against the actual GitHub assets and their `checksums.txt`; exact URLs and
SHA-256 pins are in `cli/resources/cua/release.json`. The signed extension's
source, CRX, version, ID and checksum are in `cli/resources/machine-browser/`;
CI checks deterministic packaging and its signature. No perception extension
is installed, and telemetry is disabled.

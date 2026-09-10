# NyxID Oracle CDP worker

The CDP worker drives a dedicated, logged-in Chrome profile through the Chrome
DevTools Protocol. It implements the same NyxID worker API as the unchanged
[Tampermonkey userscript](../nyxid_oracle.user.js), with process supervision,
crash recovery, manager commands, verified upgrades, and pool-wide login
import.

## Install with the CLI

Install Node 18 or newer, npm, and Chrome or Chromium. Log in to the NyxID CLI,
then run:

```bash
nyxid oracle worker install --pool <pool-slug> [--label <name>]
```

Active Member/Admin users can join their organization's org-visible pool using
their NyxID login. Pool owners and org admins can also join pools they manage.
The command enrolls automatically with no shared-token prompt. Complete ChatGPT
login in the dedicated Chrome window; this worker contributes its own account
to the pool's shared queue.

Existing pool-token installations retain their configuration. For a new
manager-operated worker that should receive shared saved logins, pass
`--worker-token-file <path>` or `--login-profile <name>` during install. Saved
profiles require the raw pool token. Use a separate `--profile` for shared-login
workers alongside an automatically enrolled installation.

Install performs these actions:

1. Reserves a unique label (server-generated or `--label <name>`). Automatic
   enrollment refuses labels used by any other worker. Pool-token installs
   retain the legacy label-adoption path.
2. Downloads the worker source embedded in the NyxID backend and verifies its
   SHA-256.
3. Installs the exact `playwright-core` version from the bundle manifest without
   a bundled browser.
4. Writes mode `0600` config and token files, a stable installation ID, and a
   state-file path under `~/.nyxid-oracle/<pool>/`.
5. Installs a launchd LaunchAgent on macOS or a systemd user unit on Linux.
6. Starts a dedicated Chrome profile and the supervised worker.

Named NyxID CLI profiles install under
`~/.nyxid-oracle/<pool>/profiles/<profile>/`. Use the same `--profile` value on
later local commands.

Manage the service with:

```bash
nyxid oracle worker start --pool <pool>
nyxid oracle worker stop --pool <pool>
nyxid oracle worker status --pool <pool>
nyxid oracle worker logs --pool <pool> --follow
nyxid oracle worker uninstall --pool <pool>
```

`uninstall` removes the service definition but retains the installation files,
Chrome profile, and token.

## Log in shared-account workers remotely

This flow applies to manager-operated pool-token installations. Automatically
enrolled workers use their own local ChatGPT accounts and cannot receive these
shared logins.

Run this command on one machine where you can complete the ChatGPT login:

```bash
nyxid oracle login <pool> [--worker-token-file <path>]
```

The CLI opens a local dedicated Chrome profile. Password, OTP, SSO, and
Cloudflare steps happen in that local window. After ChatGPT reports an
authenticated DOM, the CLI captures allowlisted ChatGPT and OpenAI cookies and
storage. It encrypts the capture locally with AES-256-GCM and a key derived by
HKDF-SHA256 from the raw pool worker token.

After upload, the CLI stops the capture Chrome and deletes its temporary
profile, worker files, and plaintext capture. The cleanup guard also runs when
capture or upload fails.

The backend stores only the encrypted envelope, wraps it with its normal
at-rest encryption, and expires it after one hour. It queues `session_import`
only for workers that advertise support. Each worker decrypts locally, imports
through CDP after its current task, reloads ChatGPT, and verifies the DOM. The
CLI prints a result for every target worker and fails if any import does not
verify.

ChatGPT may bind a session to device or risk context. A rejected import reports
`session_import_verification_failed`; it does not claim success after cookie
injection alone.

## Inspect and control workers

```bash
nyxid oracle worker list <pool>
nyxid oracle worker show <pool> <label>
nyxid oracle worker drain <pool> <label>
nyxid oracle worker resume <pool> <label>
nyxid oracle worker restart <pool> <label>
nyxid oracle worker relaunch-browser <pool> <label>
nyxid oracle worker relogin <pool> <label>
nyxid oracle worker forget <pool> <label> [--force]
nyxid oracle worker cancel-command <pool> <label> <command-id>
nyxid oracle worker upgrade --pool <pool> [--label <label>]
```

Commands travel through worker heartbeats. The worker has no inbound listener.
Members can list and manage their own contributed workers; org admins manage
all workers. Removing membership or changing it to Viewer revokes the worker's
access. Pool-token rotation also invalidates enrollment. Once access is restored,
run `worker install --force --pool <pool>` with the original profile to renew
automatically, retaining the browser account and worker label. Forgetting an
enrolled worker revokes its credential and frees its enrollment slot. Pools
allow up to 256 enrolled installations; `max_workers` separately limits task
concurrency.

Drain, restart, browser relaunch, session import, and upgrade wait for the
current task unless a logged-out task needs an immediate session import to
continue. Command IDs and terminal results persist locally, so a delivery lease
retry does not repeat a completed side effect.

Without `--label`, upgrade targets the installed local profile and waits until
the local files and restarted worker report the expected version. With
`--label`, the CLI queues the same upgrade asynchronously for a remote worker.
The worker installs the exact `playwright-core` version with a five-minute
timeout, verifies the backend-embedded source SHA-256, replaces `worker.mjs`,
and exits. launchd or systemd starts the new bundle.

## Recovery behavior

The worker treats NyxID network failures, Chrome failure, and tab failure as
recoverable conditions:

- HTTP timeouts, rate limits, and server errors retry with capped exponential
  backoff and jitter. A transient fetch failure does not exit the worker.
- A CDP disconnect triggers reconnect. Repeated failures relaunch Chrome with
  the configured executable, profile, and debug port.
- Repeated task-level browser failures stop after a bounded count. A known
  pre-send failure consumes a server infrastructure retry. A post-send failure
  returns `prompt_delivery_uncertain` and never replays the prompt.
- A closed, crashed, or navigated-away tab is replaced with `chatgpt.com`. The
  worker restores the server-provided project URL.
- The mode `0600` state file records only the task ID, attempt ID, conversation
  URL, phase, and transcript baseline. It never stores prompts, responses,
  transcripts, cookies, storage, or signed image URLs.
- Before clicking Send, the worker persists `send_attempted`. After recovery it
  checks the transcript after the saved baseline. It extracts a completed
  answer, waits for an existing pending turn, or sends only from a known
  pre-send phase. An uncertain post-send state fails with
  `prompt_delivery_uncertain` and never resends the prompt.

The server requeues an expired lease to the FIFO front while the task has
infrastructure retries left. New tasks default to three retries. Task status
reports both fresh dispatch attempts and retries.

### Debug-port collisions

The supervised worker deliberately keeps one debug port in its persisted
configuration. It does not probe and attach to another local CDP endpoint
because that would diverge from the launchd or systemd environment and could
connect the worker to an unrelated browser profile.

If another process later takes that port, `nyxid oracle worker list <pool>`
shows `Chrome` as `no`; `nyxid oracle worker show <pool> <label>` shows the
sanitized connection error. Close the dedicated NyxID Chrome window if it is
still open without CDP, then run:

```sh
nyxid oracle worker install --force --pool <pool>
```

Add the same `--profile <name>` used for the original installation. Forced
installation retains the worker label, token, and Chrome profile; it keeps the
port when a live Chrome still answers CDP there and probes a new free port only
when it does not, rewrites `CHROME_CDP_URL` and `CHROME_DEBUG_PORT` together,
and restarts the supervisor. Running `install --force` is therefore also the
safe way to refresh an existing install's service environment without
disturbing its logged-in Chrome.

## Manual setup

The CLI install is the supported path. For development, run the worker from this
directory:

```bash
npm install
./start-chrome.sh

umask 077
printf '%s' 'nyx_owk_xxxxxxxx' > ~/.nyxid-oracle-token
NYXID_BASE_URL=https://auth.nyxid.dev \
NYXID_WORKER_TOKEN_FILE="$HOME/.nyxid-oracle-token" \
NYXID_WORKER_LABEL=dev-worker-1 \
NYXID_CHROME_EXECUTABLE="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
node worker.mjs
```

Use a different worker label, debug port, and Chrome profile for each concurrent
worker.

## Reasoning level

The pool's `--model` (or a task's `model_label`) requests a ChatGPT reasoning
level: `chatgpt-6-pro` and `chatgpt-5.5-pro` request **Pro**; `extra high`,
`high`, `medium`, and `instant` request those levels. Existing Chinese aliases
remain supported. Picker discovery uses the structural composer pill, even
when its label is unfamiliar (for example `自动`, `Auto`, or `6`). If that
pill is absent, only menu buttons inside the textarea's nearest form, or its
nearest ancestor containing Send, are considered. Header/account menus are
never picker fallbacks. Among multiple structural pills or fallback buttons,
a recognized reasoning level takes priority, then the existing label hints,
then the first candidate. An unfamiliar single pill remains eligible.

Selection uses real pointer clicks and exact level matches before fuzzy
matches. High cannot match Extra High. A sticky submenu can commit only a
recognized target entry (exact, then fuzzy), or a recognized checked level;
otherwise it is dismissed with Escape. There is no first-item fallback.
Before opening the pill, the worker clears a leftover Radix body pointer-events
lock with at most three Escape presses, then records the identity of menus
already visible. An unrelated sidebar menu never triggers this lock cleanup.
Picker waits, item/effort-trigger selection, and cleanup consider only newly
visible menus, so a persistent sidebar listbox is ignored. Cleanup
uses at most three Escape presses per call. Selected entries are revalidated
by visible text and picker membership, then clicked through their exact
element handle; hidden hints in an item's text content do not affect matching.

Selection returns `{ level, verified, observed, reason }`. Verification means
the observed composer pill shows the requested level. The result's `model`
reports that observed pill text even when unverified; if no pill text could
be read, it retains the requested model as a fallback, not as evidence of a
successful selection. Clicked menu text is never reported as the model.

Selection is best-effort, with a shared 25-second deadline (shortenable with
`NYXID_MODEL_SELECT_TIMEOUT_MS`). Every step checks the remaining budget.
Picker clicks and key presses allow up to three seconds, reads one second,
and menu opening five seconds, each capped by the remaining budget. Actions
carry an abort signal. `timeout` means the shared deadline/abort was reached;
a shorter menu wait reports `menu_not_opened`, other step failures report
`selection_failed`, and an expired DOM read reports `interaction_deadline`.
On deadline expiry the worker aborts, allows up to three seconds to drain
the inner operation, then spends at most two seconds closing menus. No
background picker loop continues into prompt delivery, and selection errors
do not consume browser recovery attempts.
The initial composer visibility wait allows 60 seconds for slow page loads;
click/fill/Send actions remain bounded to five seconds. Before typing and
before Send, a separate five-second guard scrolls the composer into view,
checks its hit target and body pointer events, and dismisses obstructions.
If Escape leaves it blocked, the guard can click neutral main padding even
when a sidebar listbox is visible. An unrelated menu does not fail the check.
A persistent obstruction raises `composer_unobstructed_failed` into existing
pre-send browser recovery.
After the local recovery budget is exhausted, `browser_recovery_exhausted`
lets the server requeue the task while infrastructure retries remain. Logs
report `browser failure <n>/<max> (<code>)` for each failure; `paused for
browser recovery` appears only when another local recovery will run.

Progress acknowledgements run `page_ready` → `selecting_model` →
`ready_to_send` → `sent`. A second `selecting_model` acknowledgement records
the finished selection's metadata-only `phase_detail`, such as `selected=Pro`,
`unverified=Pro`, `picker_unavailable`, `level_unavailable`, `menu_not_opened`,
`selection_failed`, `interaction_deadline`, or `timeout`. `ready_to_send`
refreshes the lease after filling the prompt; first-turn uploads acknowledge
it again before Send. Pre-send cancellation replies stop delivery.
Acknowledgements include `page_url` for task/worker protocol diagnostics;
`phase_detail` stays metadata-only. Conversation URLs
remain excluded from logs and audit, not from the worker protocol.

Selection logs include `model_selection reason=<code>`, pill source
(`structural`/`fallback`/`none`), detected level or `unrecognized`, pill text
length, the last visible picker item count, and recognized canonical levels
(e.g. `items=5 recognized=[Instant,Medium,High,Extra High,Pro]`). They never
include raw pill/menu labels, prompts, answers, or conversation URLs. The
durable `send_attempted` fence and the rule against resending an uncertain
prompt remain in force.

## Result artifacts

For prompt tasks, the worker inspects only the last assistant turn. It captures
generated images plus download links hosted by ChatGPT's `/backend-api/`
content endpoints or `*.oaiusercontent.com`. Page-local ChatGPT `blob:` URLs
are also supported. Model-produced URLs remain untrusted: the allowlist is
checked again before the cookie-bearing request and on every redirect. Links
already represented by a generated image are deduplicated by ChatGPT file ID.

File names use the `download` attribute, anchor text, or URL in that order and
are sanitized to a safe 128-character basename. The worker downloads at most
four images and eight files, up to 6 MiB each, under one 9 MiB decoded-byte
budget. The server revalidates every payload. Logs report only artifact counts
and byte sizes, never file names or bodies.

Use `nyxid oracle ask --artifacts <dir>` or
`nyxid oracle result <task-id> --artifacts <dir>` to save all artifacts. The
server returns their base64 bodies in JSON and retains them on the task until
its normal `ORACLE_TASK_RETENTION_DAYS` expiry. `--out` continues to save images
only. The deployed userscript is unchanged and simply omits generic files.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `NYXID_BASE_URL` | required | NyxID server base URL. |
| `NYXID_WORKER_TOKEN_FILE` | none | Path to the private installation credential or shared pool token file. Managed by CLI installs. |
| `NYXID_WORKER_TOKEN` | none | Inline token fallback. This can appear in shell history and process environments. |
| `NYXID_WORKER_LABEL` | `tab_1` | Worker identity within the pool. CLI installs allocate this value. |
| `NYXID_WORKER_STATE_FILE` | `~/.nyxid-oracle/worker-state.json` | Durable recovery and command journal. |
| `NYXID_INSTALLATION_ID_FILE` | beside the state file | Stable installation identity used to bind an allocated label. |
| `NYXID_BUNDLE_VERSION_FILE` | beside `worker.mjs` | Installed bundle version. The worker accepts it only when its hash suffix matches the running source. |
| `CHROME_CDP_URL` | `http://localhost:9222` | Chrome DevTools endpoint. |
| `CHROME_DEBUG_PORT` | CDP URL port or `9222` | Port used when the worker relaunches Chrome. |
| `CHROME_PROFILE_DIR` | `~/.nyxid-oracle/chrome-profile` | Dedicated Chrome profile. |
| `NYXID_CHROME_EXECUTABLE` | none | Chrome or Chromium executable used for recovery. Without it the worker can reconnect but cannot relaunch Chrome. |
| `NYXID_CHROME_ARGS_JSON` | none | JSON string array of extra Chrome arguments. |
| `NYXID_POLL_MS` | `5000` | Idle task-poll interval. |
| `NYXID_PRESENCE_MS` | `20000` | Presence heartbeat interval. |
| `NYXID_HTTP_TIMEOUT_MS` | `30000` | Per-request timeout. |
| `NYXID_MODEL_SELECT_TIMEOUT_MS` | `25000` | Reasoning selection deadline, clamped to 1–25000 ms; abort/drain and menu cleanup follow it. |
| `NYXID_MAX_HTTP_BACKOFF_MS` | `60000` | Maximum network retry delay. |
| `NYXID_MAX_CDP_FAILURES_BEFORE_RELAUNCH` | `3` | CDP failures before a full Chrome relaunch. |
| `NYXID_MAX_TASK_RECOVERY_FAILURES` | `6` | Task-level browser failures before the worker reports a bounded failure. |
| `NYXID_NPM_EXECUTABLE` | `npm` | npm executable used by pushed upgrades. |
| `NYXID_NPM_INSTALL_TIMEOUT_MS` | `300000` | Maximum dependency-install time during a pushed upgrade. |
| `NYXID_MAX_WAIT_MS` | `7200000` | Maximum answer wait. |
| `NYXID_STABLE_INTERVAL_MS` | `8000` | Response stability poll interval, clamped to 100–60000 ms. Shorter intervals also shorten the completion stability window; browser fixtures use 500 ms. |
| `NYXID_NO_OUTPUT_IDLE_MS` | `420000` | Non-generating wait before an empty answer fails. |

## Security boundaries

- The Chrome debug port is an unauthenticated local control channel. Keep it on
  loopback and use a dedicated Chrome profile. Do not reuse that profile for
  unrelated sensitive logins.
- The CLI stores its installation credential privately with mode `0600`; the
  backend binds it to one worker and the contributing user's current membership.
  Shared pool tokens retain their existing broader access. Rotate the pool token
  if it leaks, update pool-token workers and userscripts, and renew enrolled
  workers with `install --force`.
- The state file contains no session or task bodies. Worker logs use stable
  error codes and task metadata. They do not print prompts, responses,
  transcripts, cookies, storage, raw tokens, conversation URLs, attachment
  filenames, signed image URLs, generated file bodies, or generated filenames.
- The backend sees a raw bearer token while authenticating a live worker
  request. The login-envelope claim applies to persisted server state: the
  stored worker-token hash cannot derive the HKDF key.
- The `extract` task kind drives the real logged-in browser. The server and
  worker reject loopback, private, link-local, metadata, and rebinding targets.
  Keep `allow_extract` disabled unless every pool submitter may read from that
  browser's network position.

## Tests

```bash
node --check worker.mjs
node --test worker.test.mjs
```

## One tab per worker

The worker drives exactly one ChatGPT tab in its dedicated Chrome. On every
reconnect it reuses the existing ChatGPT tab and closes duplicate ChatGPT tabs
that earlier recoveries left behind (a login-flow tab is left alone). A
browser relaunch (`relaunch-browser`, or automatic after repeated CDP
failures) first stops the Chrome process bound to this profile and then starts
it again, so it never hands a URL to a still-running instance and adds a tab.
Chrome is launched without a start URL; the worker opens ChatGPT itself.

## Identifying the dedicated Chrome

Each install (and each `--profile`) drives its own Chrome user-data-dir under
`~/.nyxid-oracle/<pool>/…/chrome-profile`. The profile is named
`NyxID Oracle <pool>` (CLI launches) / `NyxID Oracle <label>` (worker
relaunches) so the window's profile avatar menu and `chrome://version`
(Profile Path) show which pool/worker it serves. `nyxid oracle worker status
--pool <pool>` prints the same paths and the CDP port.

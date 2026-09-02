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

The command asks for the raw pool worker token with hidden input. Pass
`--worker-token-file <path>` to read it from a file instead. The server cannot
return an existing token because it stores only the SHA-256 hash.

Install performs these actions:

1. Allocates a label that is unique within the pool (server-generated, or
   `--label <name>` to keep your own naming; an existing legacy worker's
   label is adopted, a label bound to another managed install is refused).
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

## Log in every worker remotely

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
nyxid oracle worker upgrade --pool <pool> [--label <label>]
```

Commands travel through worker heartbeats. The worker has no inbound listener.
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

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `NYXID_BASE_URL` | required | NyxID server base URL. |
| `NYXID_WORKER_TOKEN_FILE` | none | Preferred path to the pool token file. |
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
| `NYXID_MAX_HTTP_BACKOFF_MS` | `60000` | Maximum network retry delay. |
| `NYXID_MAX_CDP_FAILURES_BEFORE_RELAUNCH` | `3` | CDP failures before a full Chrome relaunch. |
| `NYXID_MAX_TASK_RECOVERY_FAILURES` | `6` | Task-level browser failures before the worker reports a bounded failure. |
| `NYXID_NPM_EXECUTABLE` | `npm` | npm executable used by pushed upgrades. |
| `NYXID_NPM_INSTALL_TIMEOUT_MS` | `300000` | Maximum dependency-install time during a pushed upgrade. |
| `NYXID_MAX_WAIT_MS` | `7200000` | Maximum answer wait. |
| `NYXID_NO_OUTPUT_IDLE_MS` | `420000` | Non-generating wait before an empty answer fails. |

## Security boundaries

- The Chrome debug port is an unauthenticated local control channel. Keep it on
  loopback and use a dedicated Chrome profile. Do not reuse that profile for
  unrelated sensitive logins.
- Treat the worker token as a long-lived pool credential. Prefer a mode `0600`
  token file. Rotate the pool token if it leaks, then update every installed
  worker and userscript.
- The state file contains no session or task bodies. Worker logs use stable
  error codes and task metadata. They do not print prompts, responses,
  transcripts, cookies, storage, raw tokens, conversation URLs, attachment
  filenames, or signed image URLs.
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

## Identifying the dedicated Chrome

Each install (and each `--profile`) drives its own Chrome user-data-dir under
`~/.nyxid-oracle/<pool>/…/chrome-profile`. The profile is named
`NyxID Oracle <pool>` (CLI launches) / `NyxID Oracle <label>` (worker
relaunches) so the window's profile avatar menu and `chrome://version`
(Profile Path) show which pool/worker it serves. `nyxid oracle worker status
--pool <pool>` prints the same paths and the CDP port.

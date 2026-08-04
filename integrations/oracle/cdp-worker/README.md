# NyxID Oracle CDP worker

A lower-friction alternative to the [Tampermonkey userscript](../nyxid_oracle.user.js):
instead of installing a browser extension and keeping a tab babysat, this
attaches to your **already-running, already-logged-in Chrome** over the Chrome
DevTools Protocol and drives the ChatGPT tab for you, as a background daemon.

It speaks the exact same NyxID worker API (`/api/v1/oracle/worker/*`) and reuses
the same proven answer extraction (KaTeX→LaTeX, Pro-reasoning completion
detection, full-transcript scrape), so **no NyxID backend change is needed** —
it's a drop-in replacement for the userscript's browser side.

Because it drives your **real** Chrome (your real session and TLS fingerprint,
the Cloudflare clearance you already earned by logging in normally), it's far
less bot-detectable than a fresh headless browser.

## Setup (two commands)

Prereqs: Node 18+ and a NyxID oracle pool worker token
(`nyxid oracle pool create … --output json` prints `worker_token`).

```bash
cd integrations/oracle/cdp-worker
npm install            # installs playwright-core only (no bundled browser)

# 1. Launch Chrome with a debug port + a dedicated profile, then log into
#    ChatGPT once in the window that opens (the login persists):
./start-chrome.sh

# 2. Store the worker token in a file (keeps this long-lived credential out of
#    your shell history and the process environment), then run the worker:
umask 077 && printf '%s' 'nyx_owk_xxxxxxxx' > ~/.nyxid-oracle-token
NYXID_BASE_URL=https://auth.nyxid.dev \
NYXID_WORKER_TOKEN_FILE=~/.nyxid-oracle-token \
NYXID_WORKER_LABEL=tab_1 \
node worker.mjs
```

(For a quick test you may still pass `NYXID_WORKER_TOKEN=nyx_owk_…` inline, but
that lands in shell history and `ps e` / `/proc/<pid>/environ` — prefer the file.)

That's it. The worker polls NyxID for tasks, types prompts into ChatGPT, waits
for the answer (including long Pro reasoning), and posts results back. Consumers
call it exactly as before:

```bash
nyxid oracle ask <pool> "your question"
nyxid oracle attach <pool> https://chatgpt.com/c/<uuid>
```

## Configuration (env vars)

| Var | Default | Meaning |
|-----|---------|---------|
| `NYXID_BASE_URL` | — (required) | NyxID server, e.g. `https://auth.nyxid.dev` |
| `NYXID_WORKER_TOKEN_FILE` | — | Path to a file holding the pool worker token (`nyx_owk_…`). **Preferred** over the inline var — keeps the credential out of shell history and the process environment. |
| `NYXID_WORKER_TOKEN` | — | Pool worker token, passed inline. Used only if `NYXID_WORKER_TOKEN_FILE` is unset. One of the two is required. |
| `NYXID_WORKER_LABEL` | `tab_1` | Per-worker identity; run several with **distinct** labels for more capacity. Two workers sharing one label on the same pool will steal each other's task leases — keep labels unique per pool. |
| `CHROME_CDP_URL` | `http://localhost:9222` | Where Chrome's DevTools endpoint is |
| `NYXID_POLL_MS` | `5000` | Poll interval |
| `NYXID_MAX_WAIT_MS` | `7200000` | Max wait per answer (2h) |

Multiple workers = more throughput: launch one Chrome debug instance and run
several `worker.mjs` with `NYXID_WORKER_LABEL=tab_1`, `tab_2`, … (up to the
pool's `max_workers`). Each can target a different Chrome window/profile via
`CHROME_CDP_URL` if you want true parallelism.

## How it compares

| | Userscript | **CDP worker** |
|---|---|---|
| Install | Tampermonkey + script | `npm install` (playwright-core) |
| Browser | any logged-in tab | your real Chrome on a debug port |
| Babysitting | keep a tab open & active | runs as a daemon |
| Detection risk | lowest (in-page) | low (real session, CDP-driven) |
| Backend change | none | none |

The userscript remains the zero-dependency option (nothing to run locally). The
CDP worker is the low-friction option once you're willing to run a small Node
process. Both can serve the same pool.

## Security & trust boundaries

- **The Chrome debug port is an unauthenticated control channel.**
  `--remote-debugging-port` exposes a DevTools endpoint that gives *any local
  process which can reach it* full control of this Chrome profile (its ChatGPT
  session and cookies). `start-chrome.sh` binds it to localhost (it does **not**
  pass `--remote-debugging-address=0.0.0.0`) and uses a dedicated
  `--user-data-dir`, which are the right mitigations — **do not** widen the bind
  address and **do not** reuse this profile for other sensitive logins. On a
  shared host, consider `--remote-debugging-pipe` instead of a TCP port.
- **Worker token = a long-lived pool credential.** Prefer
  `NYXID_WORKER_TOKEN_FILE` (chmod 600) over the inline env var; rotate it with
  `nyxid oracle pool rotate-token` if it leaks.
- **`extract` (read any web page) is opt-in per pool and off by default.** It
  drives this real logged-in browser, so the server blocks
  loopback/private/link-local/cloud-metadata targets and the worker re-checks at
  navigation time (DNS-rebinding defense). Enable it only on pools you trust the
  submitters of: `nyxid oracle pool update <slug> --allow-extract true`. See
  `docs/ORACLE_RELAY.md` for the full blast-radius discussion.

## Limitations (v1)

- PDF attachments aren't handled yet (text prompts + transcript scrape are).
- Designed for one ChatGPT account per Chrome profile; use separate
  `CHROME_PROFILE_DIR` + `CHROME_CDP_URL` for multiple accounts.
- ChatGPT DOM changes can still break extraction; the heuristics mirror the
  userscript's and are updated there first.

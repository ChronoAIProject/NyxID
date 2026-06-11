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

# 2. In another terminal, run the worker:
NYXID_BASE_URL=https://auth.nyxid.dev \
NYXID_WORKER_TOKEN=nyx_owk_xxxxxxxx \
NYXID_WORKER_LABEL=tab_1 \
node worker.mjs
```

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
| `NYXID_WORKER_TOKEN` | — (required) | Pool worker token (`nyx_owk_…`) |
| `NYXID_WORKER_LABEL` | `tab_1` | Per-worker identity; run several with distinct labels for more capacity |
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

## Limitations (v1)

- PDF attachments aren't handled yet (text prompts + transcript scrape are).
- Designed for one ChatGPT account per Chrome profile; use separate
  `CHROME_PROFILE_DIR` + `CHROME_CDP_URL` for multiple accounts.
- ChatGPT DOM changes can still break extraction; the heuristics mirror the
  userscript's and are updated there first.

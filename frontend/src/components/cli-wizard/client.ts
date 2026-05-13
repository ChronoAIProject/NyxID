/**
 * Mode-A fetch interceptor for the locally-served wizard bundle.
 *
 * The shared confirm panels (`ai-key-confirm-panel.tsx`, `confirm-panels.tsx`,
 * `auth-flows.tsx`) and React-Query hooks (`use-keys`, `use-nodes`) all
 * call the backend via the `/api/v1/...` path against the regular
 * `api-client.ts`, which assumes same-origin + cookie session. That
 * contract holds for Mode B (`/cli/pair` served by the frontend at
 * port 3000), but NOT for Mode A — the CLI's embedded axum server at
 * `127.0.0.1:<port>` doesn't carry the user's cookie and isn't the
 * backend origin. Mode A's server exposes a CSRF-protected catch-all
 * proxy at `/api/proxy/<rest>` that attaches the user's bearer token
 * server-side; the browser needs to route all `/api/v1/*` calls
 * through that prefix.
 *
 * This module installs a one-shot `window.fetch` shim that:
 *
 * 1. Rewrites `/api/v1/<path>` → `/api/proxy/api/v1/<path>` for every
 *    request (so every shared component keeps calling `/api/v1/*`
 *    without knowing which mode it runs in).
 * 2. Attaches the `x-wizard-csrf` header — Mode A's server rejects
 *    anything missing it.
 * 3. Short-circuits `/api/v1/cli-pairings/*` calls with synthetic
 *    success responses. Mode A has no pairing record; the shared
 *    confirm panels call `reservePairingAction` / `withRewindOnError`
 *    for Mode B bookkeeping, and those are no-ops here.
 * 4. Rewrites `DELETE /api/v1/keys/<id>?only_if_pending=true`
 *    (placeholder cleanup for ai-key OAuth / device-code abandon) to
 *    Mode A's wizard-server-local
 *    `POST /api/proxy/abandon-placeholder` endpoint. The server does
 *    a conditional GET-then-DELETE so a key that just flipped to
 *    `active` isn't accidentally revoked.
 *
 * Install by calling `installModeAFetchShim(bootstrap)` once before
 * rendering the React root. Idempotent — subsequent calls replace the
 * bootstrap (in case of hot-reload during dev).
 */

export interface WizardBootstrap {
  /** Flow kind — maps to `PairingFlow` / `FlowKind`. */
  readonly flow:
    | "ai-key"
    | "api-key-create"
    | "api-key-rotate"
    | "node-register-token"
    | "node-rotate-token"
  /** CSRF token minted by the wizard server. */
  readonly csrf: string
  /** Backend base URL. Opaque here — used by the server to build proxy URL. */
  readonly baseUrl: string
  /** "local" for Mode A. */
  readonly context: "local"
  /** Per-flow prefill blob (shape varies). */
  readonly prefill?: Record<string, unknown>
}

let installed = false
let currentBootstrap: WizardBootstrap | null = null

export function installModeAFetchShim(bootstrap: WizardBootstrap): void {
  currentBootstrap = bootstrap
  if (installed) return
  installed = true

  const originalFetch = window.fetch.bind(window)

  window.fetch = async (
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    const req = toRequest(input, init)
    const url = new URL(req.url, window.location.origin)

    // Only intercept same-origin requests that start with /api/v1.
    if (url.origin !== window.location.origin) {
      return originalFetch(req)
    }

    // (3) Pairing bookkeeping calls. Mode A doesn't have a pairing
    // record, but the shared confirm panels still call
    // `reservePairingAction` / `withRewindOnError` (at-most-once
    // guarantee) and the ai-key sub-flow registers `beforeunload`
    // handlers that fire `/cli-pairings/{id}/cancel` with
    // `keepalive: true` so the user's tab-close is noticed.
    //
    // Routing strategy:
    //   - `/cancel` (unload path) → forward to Mode A's
    //     `/api/proxy/cancel-unload` so the CLI actually exits
    //     promptly instead of waiting for the heartbeat watchdog
    //     (22-60s) to reap the session.
    //   - everything else (reserve-action, rewind-action, complete
    //     during happy-path) → synthetic 200. The Mode A happy path
    //     fires its own `/api/proxy/complete` on DisplayOnce ack via
    //     `postWizardComplete` in this module, so the shared
    //     `/cli-pairings/{id}/complete` call is redundant and harmless
    //     to no-op. (Known limitation: if the user closes the tab
    //     AFTER the OAuth / device-code callback activates the
    //     service but BEFORE the polling loop notices, the shared
    //     keepalive `/cli-pairings/{id}/complete` is swallowed and the
    //     CLI falls back to the heartbeat timeout. See issue
    //     <follow-up> for a proper ack-shape-aware passthrough.)
    if (url.pathname.startsWith("/api/v1/cli-pairings/")) {
      if (url.pathname.endsWith("/cancel")) {
        return originalFetch("/api/proxy/cancel-unload", {
          method: "POST",
          headers: withCsrf({ "content-type": "application/json" }),
          body: "{}",
          keepalive: init?.keepalive ?? false,
        })
      }
      return new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { "content-type": "application/json" },
      })
    }

    // (4) Placeholder cleanup path. `useAbandonPlaceholder` fires
    // `DELETE /api/v1/keys/<id>?only_if_pending=true` on OAuth /
    // device-code abandonment; Mode A must route to the server-local
    // abandon-placeholder handler instead (it does the conditional
    // revoke atomically using its own bearer token).
    if (
      req.method === "DELETE" &&
      /^\/api\/v1\/keys\/[^/]+$/.test(url.pathname) &&
      url.searchParams.get("only_if_pending") === "true"
    ) {
      const keyId = url.pathname.split("/").pop() ?? ""
      return originalFetch("/api/proxy/abandon-placeholder", {
        method: "POST",
        headers: withCsrf({ "content-type": "application/json" }),
        body: JSON.stringify({ key_id: keyId }),
      })
    }

    // (1) Rewrite `/api/v1/*` → `/api/proxy/api/v1/*`.
    if (url.pathname.startsWith("/api/v1/")) {
      const rewritten = new URL(url.toString())
      rewritten.pathname = `/api/proxy${url.pathname}`
      // (2) Add CSRF header.
      const headers = withCsrf(extractHeaders(req, init))
      return originalFetch(rewritten.toString(), {
        method: req.method,
        headers,
        body: await readBody(req),
        credentials: req.credentials,
        signal: req.signal,
      })
    }

    return originalFetch(req)
  }
}

/**
 * POST the final ack payload back to the wizard server so the CLI's
 * blocking `run_flow` future resolves. Mode-A-only; Mode B uses
 * `/cli-pairings/{id}/complete` instead.
 */
export async function postWizardComplete(
  body: Record<string, unknown>,
): Promise<void> {
  const response = await fetch("/api/proxy/complete", {
    method: "POST",
    headers: withCsrf({ "content-type": "application/json" }),
    body: JSON.stringify(body),
  })
  if (!response.ok) {
    throw new Error(
      `/api/proxy/complete failed: ${String(response.status)} ${response.statusText}`,
    )
  }
}

/**
 * POST to /api/proxy/cancel so a user abandon (e.g. "Cancel" button)
 * surfaces as WizardOutcome::Cancelled on the CLI side.
 */
export async function postWizardCancel(): Promise<void> {
  try {
    await fetch("/api/proxy/cancel", {
      method: "POST",
      headers: withCsrf({ "content-type": "application/json" }),
      body: "{}",
    })
  } catch {
    // Best-effort — CLI also has an unload handler and heartbeat
    // watchdog so a failed cancel still terminates the wizard.
  }
}

/** How often the browser pings the CLI's wizard server. */
const HEARTBEAT_INTERVAL_MS = 1200
/**
 * Consecutive heartbeat failures before the UI flips to
 * "disconnected". 3 × 1200ms = ~3.6s detection window. Fast enough
 * to feel snappy, loose enough to tolerate one hiccup.
 *
 * The CLI's server-side watchdog is intentionally more generous:
 * it waits for the first successful heartbeat, then allows a longer
 * active miss window before cancelling. This UI warning is only a
 * quick signal that heartbeat checks are currently failing.
 */
const DISCONNECT_THRESHOLD = 3

/**
 * Emit periodic heartbeats to `/api/proxy/heartbeat` so the CLI's
 * watchdog keeps the server alive while this tab is open.
 *
 * Also runs the reverse direction: the browser watches for heartbeat
 * failures (connection refused, 5xx) and calls `onDisconnect` after
 * three consecutive misses so the UI can surface "CLI has gone away"
 * instead of letting the user interact with a zombie tab.
 * `onReconnect` fires once a heartbeat lands again (in case the CLI
 * was paused / suspended and resumed).
 *
 * Timings are tuned for Mode A's loopback-only connection where
 * latency is measured in microseconds — network-generation-level
 * intervals (10s+) would make the disconnect UI feel sluggish.
 */
export function installHeartbeat(opts?: {
  readonly onDisconnect?: () => void
  readonly onReconnect?: () => void
}): () => void {
  let consecutiveFails = 0
  let disconnected = false
  const timer = window.setInterval(() => {
    void fetch("/api/proxy/heartbeat", {
      method: "POST",
      headers: withCsrf({ "content-type": "application/json" }),
      body: "{}",
    })
      .then((r) => {
        if (!r.ok) throw new Error(`heartbeat ${String(r.status)}`)
        consecutiveFails = 0
        if (disconnected) {
          disconnected = false
          opts?.onReconnect?.()
        }
      })
      .catch(() => {
        consecutiveFails += 1
        if (consecutiveFails >= DISCONNECT_THRESHOLD && !disconnected) {
          disconnected = true
          opts?.onDisconnect?.()
        }
      })
  }, HEARTBEAT_INTERVAL_MS)
  return () => {
    window.clearInterval(timer)
  }
}

// ── helpers ─────────────────────────────────────────────────────────

function toRequest(input: RequestInfo | URL, init?: RequestInit): Request {
  if (input instanceof Request) return input
  return new Request(input, init)
}

function extractHeaders(
  req: Request,
  init: RequestInit | undefined,
): Record<string, string> {
  const headers: Record<string, string> = {}
  req.headers.forEach((value, key) => {
    headers[key] = value
  })
  if (init?.headers) {
    const incoming = new Headers(init.headers)
    incoming.forEach((value, key) => {
      headers[key] = value
    })
  }
  return headers
}

function withCsrf(base: Record<string, string>): Record<string, string> {
  if (!currentBootstrap) return base
  return { ...base, "x-wizard-csrf": currentBootstrap.csrf }
}

async function readBody(req: Request): Promise<BodyInit | null> {
  if (req.method === "GET" || req.method === "HEAD") return null
  try {
    const text = await req.clone().text()
    return text.length > 0 ? text : null
  } catch {
    return null
  }
}

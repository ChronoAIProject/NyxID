/**
 * Shared "verify AI service" probe used by the aha ConnectVerifyStep
 * dialog and the standalone VerifyKeyCard on api-key detail.
 *
 * ## Why this file exists
 *
 * The catalog ships ~29 seeded providers. The previous per-call probe
 * only mapped 7 openai-family slugs to `/v1/models`; the other 22
 * fell through to `""` (the proxy root) and got wildly-varying
 * results — HTML pages, 404, or in `telegram-bot`'s case a **200
 * success even with bad credentials** — which either false-negatives
 * (user thinks their key is broken) or false-positives (user thinks
 * a broken setup works) the whole verify UX.
 *
 * ## The truth signal
 *
 * NyxID's proxy handler stamps `X-NyxID-Agent-Id` on every response
 * that made it past auth + scope + route resolution — including
 * responses where downstream returned 4xx/5xx. That header is the
 * ONE reliable signal that:
 *   1. The Agent Key is valid (auth mw accepted the bearer)
 *   2. The scope check passed (this key is allowed to call this slug)
 *   3. NyxID resolved the service + proxied through to downstream
 *
 * So "Test Agent Key" should classify success by that header, NOT by
 * whether downstream returned 2xx. The downstream response body/
 * status is diagnostic (did the stored credential actually work?)
 * but it does not affect the agent-key verdict.
 *
 * ## Probe path
 *
 * For openai-shaped APIs `/v1/models` is cheap and reliable. For
 * GitHub's PAT it's `/user`. For everything else we hit the root
 * and let the header do the classification — even a 404 body proves
 * the Agent Key worked as long as the header is there.
 */

const OPENAI_SHAPED_HINTS =
  /(openai|anthropic|claude|gemini|deepseek|groq|together|mistral|fireworks|perplexity|cohere|xai|grok)/i;

const GITHUB_PAT_HINTS = /^github/i;

/** Max wait per probe. Keeps the UI from stalling on a dead downstream. */
export const PROBE_TIMEOUT_MS = 8000;

/**
 * Pick a downstream path likely to return quickly for the given slug.
 *
 * The path is optional: even a 404 on the root still lets the caller
 * classify agent-key validity from the response header. This just
 * makes the diagnostic more actionable when we know a cheap probe
 * endpoint exists.
 */
export function probePathForSlug(slug: string): string {
  if (OPENAI_SHAPED_HINTS.test(slug)) return "v1/models";
  if (GITHUB_PAT_HINTS.test(slug)) return "user";
  return "";
}

/**
 * Coarse categorization of the downstream response. Only meaningful
 * when `agentKeyValid` is true (i.e. NyxID proxied through).
 */
export type DownstreamStatus =
  | "ok"
  | "auth_rejected"
  | "not_found"
  | "server_error"
  | "unexpected";

export interface ProbeOutcome {
  /** Raw HTTP status the fetch resolved to. `null` on network / timeout. */
  readonly httpStatus: number | null;
  /**
   * Was the response produced BY NyxID at all? False for network
   * failures, aborted timeouts, or CORS blocks (any of which would
   * mean the browser never got a status back).
   */
  readonly reachedNyxid: boolean;
  /**
   * The truth signal — `X-NyxID-Agent-Id` header present on response.
   * When true, the Agent Key is provably valid + scoped correctly +
   * NyxID reached downstream. When false, one of those three failed.
   */
  readonly agentKeyValid: boolean;
  /** Downstream classification. Meaningful only when agentKeyValid. */
  readonly downstreamStatus: DownstreamStatus;
  /** One-sentence human diagnostic explaining the outcome. */
  readonly diagnostic: string;
}

interface ProbeOptions {
  readonly bearerToken: string;
  /** Override the default per-slug probe path. */
  readonly probePath?: string;
  /** Test seam — inject a fetch impl. */
  readonly fetchImpl?: typeof fetch;
  /** Test seam — override the timeout. */
  readonly timeoutMs?: number;
}

function proxyUrl(slug: string, path: string): string {
  const suffix = path.startsWith("/") ? path.slice(1) : path;
  return `/api/v1/proxy/s/${encodeURIComponent(slug)}/${suffix}`;
}

/**
 * Probe the NyxID proxy for a given slug + bearer token. Returns a
 * rich outcome the caller can render however it wants; never throws
 * on network / HTTP errors (they land in the outcome).
 */
export async function probeAgentKey(
  slug: string,
  opts: ProbeOptions,
): Promise<ProbeOutcome> {
  const path = opts.probePath ?? probePathForSlug(slug);
  const url = proxyUrl(slug, path);
  const timeoutMs = opts.timeoutMs ?? PROBE_TIMEOUT_MS;
  const doFetch = opts.fetchImpl ?? window.fetch.bind(window);

  const controller = new AbortController();
  const timer = window.setTimeout(() => controller.abort(), timeoutMs);

  let response: Response | null = null;
  let httpStatus: number | null = null;
  try {
    response = await doFetch(url, {
      method: "GET",
      credentials: "omit",
      headers: {
        Authorization: `Bearer ${opts.bearerToken}`,
        "Content-Type": "application/json",
      },
      signal: controller.signal,
    });
    httpStatus = response.status;
  } catch {
    // Network error / abort / CORS — response stays null.
  } finally {
    window.clearTimeout(timer);
  }

  return classifyProbe(slug, response, httpStatus);
}

/**
 * Pure classifier — exported for tests to exercise the header + status
 * logic without setting up a fetch mock every time.
 */
export function classifyProbe(
  slug: string,
  response: Response | null,
  httpStatus: number | null,
): ProbeOutcome {
  if (!response || httpStatus === null) {
    return {
      httpStatus: null,
      reachedNyxid: false,
      agentKeyValid: false,
      downstreamStatus: "unexpected",
      diagnostic:
        "The probe timed out, was blocked by the browser, or lost network. Retry, or check the browser devtools network tab.",
    };
  }

  const agentKeyValid = response.headers.has("x-nyxid-agent-id");
  const reachedNyxid = true;

  if (!agentKeyValid) {
    return {
      httpStatus,
      reachedNyxid,
      agentKeyValid: false,
      downstreamStatus: "unexpected",
      diagnostic: diagnoseNyxidRejection(httpStatus, slug),
    };
  }

  const downstreamStatus = classifyDownstream(httpStatus);
  return {
    httpStatus,
    reachedNyxid,
    agentKeyValid: true,
    downstreamStatus,
    diagnostic: diagnoseDownstream(downstreamStatus, httpStatus),
  };
}

function classifyDownstream(status: number): DownstreamStatus {
  if (status >= 200 && status < 400) return "ok";
  if (status === 401 || status === 403) return "auth_rejected";
  if (status === 404) return "not_found";
  if (status >= 500) return "server_error";
  return "unexpected";
}

function diagnoseNyxidRejection(status: number, slug: string): string {
  if (status === 401) {
    return "NyxID rejected the Agent Key. The key may be revoked, expired, or missing the `proxy` scope. Rotate the key and try again.";
  }
  if (status === 403) {
    return `NyxID rejected the request as out-of-scope. This Agent Key isn't allowed to call \`${slug}\` — add the service to the key's allowlist, or use a key with \`allow_all_services\`.`;
  }
  if (status === 404) {
    return `NyxID doesn't know the slug \`${slug}\`. Was the service deleted, or is the slug spelled differently in your setup?`;
  }
  return `Unexpected NyxID response (HTTP ${String(status)}). This shouldn't happen — check the browser devtools network tab for the response body.`;
}

function diagnoseDownstream(
  status: DownstreamStatus,
  httpStatus: number,
): string {
  switch (status) {
    case "ok":
      return `End-to-end verified (HTTP ${String(httpStatus)}). NyxID accepted your Agent Key and the downstream returned success. Copy the Agent Key + Base URL above into your AI tool.`;
    case "auth_rejected":
      return `Your Agent Key works (proxied through NyxID). The downstream rejected the stored credential (HTTP ${String(httpStatus)}) — reconnect the service, or update the credential from /keys.`;
    case "not_found":
      return `Your Agent Key works (proxied through NyxID). The downstream returned HTTP ${String(httpStatus)} on the probe path — this is normal for services without a cheap status endpoint, and does not mean the key is broken.`;
    case "server_error":
      return `Your Agent Key works (proxied through NyxID). The downstream is having issues (HTTP ${String(httpStatus)}). Retry in a minute.`;
    case "unexpected":
      return `Your Agent Key works (proxied through NyxID). The downstream returned HTTP ${String(httpStatus)} — inspect the request in your AI tool to confirm the intended call succeeds.`;
  }
}

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

/** Max wait per probe. Keeps the UI from stalling on a dead downstream. */
export const PROBE_TIMEOUT_MS = 8000;

/**
 * Per-provider probe metadata. Known providers with a cheap, well-known
 * status endpoint get a real green-success test; providers explicitly
 * marked `null` (no cheap probe exists, e.g. chat-only APIs) skip the
 * probe entirely with a "verify by using the tool" hint.
 *
 * Unknown slugs (custom endpoints, brand-new catalog entries) fall
 * through to the openai-family regex + header-based classifier for a
 * safety-net check that at least tells the user whether NyxID is
 * reaching downstream at all.
 */
export interface ProbeRecipe {
  /** Path appended to the proxy base. Empty string = hit the root. */
  readonly path: string;
  /**
   * When set, downstream must return one of these to count as `ok`.
   * When omitted, any 2xx/3xx counts as `ok`. Useful for providers
   * whose "hello" endpoint always returns 200 (e.g. Discord Bot
   * v10/users/@me returns 200 with body {id, username, ...}).
   */
  readonly successStatuses?: readonly number[];
}

/**
 * Registry keys use the SEEDED `service_slug` from the backend catalog
 * (see `backend/src/services/provider_service.rs` `service_slug`
 * fields) — NOT the bare provider slug. A UserService created from
 * the "OpenAI" catalog entry ships as slug `llm-openai`; on the
 * second connect it becomes `llm-openai-2`. `recipeForSlug()` strips
 * only the trailing `-N` suffix. Keeping registry keys aligned with
 * the actual seeded slugs (rather than a bare form with an extra
 * layer of prefix-stripping in resolution) makes drift much easier
 * to spot — a typo here maps 1:1 to a missing recipe at runtime.
 */
export const PROBE_REGISTRY: Readonly<Record<string, ProbeRecipe | null>> = {
  // ─── LLM providers (llm-* prefix) ──────────────────────────────
  // OpenAI-compatible APIs — GET /v1/models is universal + cheap
  "llm-openai": { path: "v1/models" },
  "llm-anthropic": { path: "v1/models" },
  "llm-google-ai": { path: "v1/models" },
  "llm-mistral": { path: "v1/models" },
  "llm-cohere": { path: "v1/models" },
  "llm-deepseek": { path: "v1/models" },

  // Chat-only API — no cheap GET. Verify by running `codex "hello"`.
  "llm-openai-codex": null,
  // Self-hosted — requires per-instance gateway URL; no standard probe.
  "llm-openclaw": null,

  // ─── API providers (api-* prefix) ──────────────────────────────
  // GitHub — /user works with both OAuth (api-github) and PAT (api-github-pat)
  "api-github": { path: "user" },
  "api-github-pat": { path: "user" },

  // Bot APIs — dedicated bot-identity endpoints
  "api-telegram-bot": { path: "getMe" },
  "api-discord-bot": { path: "v10/users/@me" },
  "api-slack-bot": { path: "api/auth.test" },

  // OAuth user-context providers — /me variants
  "api-google": { path: "oauth2/v1/userinfo" },
  "api-spotify": { path: "v1/me" },
  "api-twitter": { path: "2/users/me" },
  "api-reddit": { path: "api/v1/me" },
  "api-twitch": { path: "helix/users" },
  "api-microsoft": { path: "v1.0/me" },
  "api-facebook": { path: "me" },
  "api-slack": { path: "api/auth.test" },
  "api-lark": { path: "open-apis/authen/v1/user_info" },
  "api-feishu": { path: "open-apis/authen/v1/user_info" },
  "api-discord": { path: "api/v10/users/@me" },

  // No suitable status endpoint / requires special auth we don't emulate
  "api-firecrawl": null, // no public status endpoint
  "api-tiktok": null, // /oauth/userinfo needs POST body
  "api-lark-bot": null, // OpenAPI probe path needs OAuth context
  "api-feishu-bot": null, // same as lark-bot

  // ─── Other seeded service_slugs ────────────────────────────────
  // AWS + GCP — SigV4 / OAuth2, no probe path we can trust here.
  "aws-cost-explorer": null,
  "api-google-cloud": null,
};

/**
 * Look up whether we have a per-provider recipe for the slug. Handles
 * repeat-connect suffixes (`openai-2`, `llm-openai-codex-7`) by
 * stripping the trailing `-N` / `-N-M` and matching the base slug.
 */
/**
 * Look up the recipe for a slug. Registry keys already carry the
 * `llm-` / `api-` / `aws-` catalog prefix, so the only rewrite we do
 * is stripping a trailing `-<digits>` suffix — that's the shape
 * NyxID uses for repeat-connect user_services (`llm-openai-2`,
 * `api-github-pat-3`).
 */
export function recipeForSlug(slug: string): ProbeRecipe | null | undefined {
  // Defensive: undefined/empty slug flows through as "unknown".
  if (!slug) return undefined;
  if (slug in PROBE_REGISTRY) return PROBE_REGISTRY[slug];
  // Strip repeat-connect suffix (`llm-openai-2` → `llm-openai`)
  const baseNoSuffix = slug.replace(/-\d+$/, "");
  if (baseNoSuffix !== slug && baseNoSuffix in PROBE_REGISTRY) {
    return PROBE_REGISTRY[baseNoSuffix];
  }
  return undefined;
}

/**
 * Is this slug KNOWN to be untestable? Returns true only when we've
 * explicitly registered the slug (or its base) as `null` in the
 * registry.
 */
export function isKnownUntestable(slug: string): boolean {
  return recipeForSlug(slug) === null;
}

/**
 * Should the UI OFFER a Test Agent Key button for this slug?
 *
 * Only true when we have an explicit, high-confidence recipe for the
 * slug (registered in PROBE_REGISTRY as a ProbeRecipe, not `null`,
 * not `undefined`). Everything else — known-untestable providers AND
 * unregistered custom endpoints — hides the Test button so we never
 * offer a probe we can't be highly confident will produce a
 * meaningful result. A misleading green / red is worse than no probe.
 *
 * Rationale (Calvin, 2026-07-01): "tests button should have high
 * degree confidence of working" — so unregistered custom slugs (base
 * URL and API shape unknown) don't get the button either. The user
 * can still verify by making one real call from their AI tool.
 */
export function isTestable(slug: string): boolean {
  const recipe = recipeForSlug(slug);
  return recipe !== null && recipe !== undefined;
}

/**
 * Pick a downstream path likely to return quickly for the given slug.
 *
 * Preference order:
 *   1. Explicit per-provider recipe from PROBE_REGISTRY → recipe.path
 *   2. Everything else → `""` (root). Callers that respect
 *      `isTestable()` won't reach here for unregistered slugs; the
 *      empty string is the safety-net path for callers that ignore
 *      the testability gate (currently none in-tree).
 */
export function probePathForSlug(slug: string): string {
  const recipe = recipeForSlug(slug);
  if (recipe) return recipe.path;
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

  // Guard against blank header values — a misconfigured reverse proxy
  // (or a bug in the axum handler) can send `X-NyxID-Agent-Id:` with
  // no value; `Headers.has()` returns true for that. We want the
  // truth signal to be the actual agent id string, so require length.
  // Header lookup is case-insensitive per WHATWG fetch spec.
  const agentIdHeader = response.headers.get("x-nyxid-agent-id");
  const agentKeyValid =
    typeof agentIdHeader === "string" && agentIdHeader.length > 0;
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
      return `Your Agent Key works (proxied through NyxID). The downstream returned HTTP ${String(httpStatus)} on the probe path — this is normal for services without a suitable status endpoint, and does not mean the key is broken.`;
    case "server_error":
      return `Your Agent Key works (proxied through NyxID). The downstream is having issues (HTTP ${String(httpStatus)}). Retry in a minute.`;
    case "unexpected":
      return `Your Agent Key works (proxied through NyxID). The downstream returned HTTP ${String(httpStatus)} — inspect the request in your AI tool to confirm the intended call succeeds.`;
  }
}

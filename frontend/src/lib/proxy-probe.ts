import { isInsufficientCreditsHttp } from "@/lib/credits-denial";
import { translateUi } from "@/i18n/use-ui-translation";

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
 * This evidence is separate from activation: a successful first proxy
 * call also requires a downstream 2xx. Provider errors retain key/scope
 * evidence and diagnostics without completing activation.
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
/** Gateway diagnostics are small; never buffer arbitrary provider responses. */
const MAX_GATEWAY_ENVELOPE_BYTES = 4096;

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
   * When omitted, any 2xx counts as `ok`. Useful for providers
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
  // Paths are RELATIVE to the seeded catalog `base_url` — the proxy
  // forms the final URL as `{base_url.trim_end('/')}/{path}`. Common
  // trap: don't repeat an API-version segment that's already in the
  // base_url (e.g. openai's base is `.../v1`, so the recipe path is
  // `models` NOT `v1/models` — otherwise the concatenated URL is
  // `.../v1/v1/models` and every probe silently 404s). Kimi flagged
  // this bug on 2026-07-01 review of the initial commit; the seeded
  // base_urls are the source of truth (see backend/src/services/
  // provider_service.rs line ~1878+).

  // ─── LLM providers (llm-* prefix) ──────────────────────────────
  // OpenAI-compatible APIs — GET {base}/models. All 6 base_urls end
  // in the provider's version segment (openai .../v1, google-ai
  // .../v1beta, cohere .../v2, etc.), so the recipe path is bare.
  "llm-openai": { path: "models" },
  "llm-anthropic": { path: "models" },
  "llm-google-ai": { path: "models" },
  "llm-mistral": { path: "models" },
  "llm-cohere": { path: "models" },
  "llm-deepseek": { path: "models" },
  // OpenRouter — GET {base}/key returns the stored key's metadata and
  // 401s on a bad credential. NOT `models`: OpenRouter's /models is
  // public (200 without auth), so it would green-light an invalid key.
  "llm-openrouter": { path: "key" },

  // Chat-only API — no cheap GET. Verify by running `codex "hello"`.
  "llm-openai-codex": null,
  // Self-hosted — requires per-instance gateway URL; no standard probe.
  "llm-openclaw": null,

  // ─── API providers (api-* prefix) ──────────────────────────────
  // GitHub — base is bare `https://api.github.com`, so `user` maps to
  // `.../user` (works with both OAuth and PAT bearer tokens).
  "api-github": { path: "user" },
  "api-github-pat": { path: "user" },
  "api-aurinko": { path: "v1/account" },

  // Bot APIs — dedicated bot-identity endpoints. Telegram Bot's path
  // is bare because the token is injected via `path` auth method
  // (adds /bot<TOKEN>/ prefix at proxy time). Discord + Slack bases
  // include /api[/v10] so the recipe strips that repeated segment.
  "api-telegram-bot": { path: "getMe" },
  "api-discord-bot": { path: "users/@me" }, // base is .../api/v10
  "api-slack-bot": { path: "auth.test" }, // base is .../api

  // OAuth user-context providers — /me variants. Recipes are relative
  // to each provider's seeded base_url (see line comments below).
  "api-google": { path: "oauth2/v1/userinfo" }, // base is bare googleapis.com
  "api-google-workspace": {
    path: "drive/v3/files?pageSize=1&fields=files(id)",
  },
  "api-google-calendar": {
    path: "calendar/v3/users/me/calendarList?maxResults=1",
  },
  "api-google-drive": { path: "drive/v3/files?pageSize=1&fields=files(id)" },
  "api-google-gmail": { path: "gmail/v1/users/me/messages?maxResults=1" },
  // Reads require an existing document ID; there is no account-level probe.
  "api-google-docs": null,
  "api-google-sheets": null,
  "api-google-slides": null,
  "api-spotify": { path: "me" }, // base is .../v1
  "api-twitter": { path: "users/me" }, // base is .../2
  "api-reddit": { path: "api/v1/me" }, // base is bare oauth.reddit.com
  "api-twitch": { path: "users" }, // base is .../helix
  "api-microsoft": { path: "me" }, // base is .../v1.0
  "api-facebook": { path: "me" }, // base is .../v21.0
  "api-linkedin": { path: "v2/userinfo" },
  "api-slack": { path: "auth.test" }, // base is .../api
  "api-lark": { path: "authen/v1/user_info" }, // base is .../open-apis
  "api-feishu": { path: "authen/v1/user_info" }, // base is .../open-apis
  "api-discord": { path: "users/@me" }, // base is .../api/v10

  // No suitable status endpoint / requires special auth we don't emulate
  "api-firecrawl": null, // no public status endpoint
  "api-tiktok": null, // /oauth/userinfo needs POST body
  "api-ifttt-mcp": null, // OAuth completion establishes the connection; tools are discovered explicitly.
  "api-ifttt": null, // Every valid operation triggers an Applet; no automatic probe.
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
  | "unexpected"
  /**
   * NyxID's own billing gate refused the request (HTTP 402
   * `insufficient_credits` envelope). Deliberately inline only: the key may
   * be pasted from another account, so this never opens the purchase dialog.
   */
  | "credits_exhausted";

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
   * Key/scope evidence: nonempty `X-NyxID-Agent-Id` matches the expected
   * selected key, and the response did not follow a redirect. Provider
   * success is evaluated separately.
   */
  readonly agentKeyValid: boolean;
  /** A headerless 403/404 with NyxID's matching error symbol and code. */
  readonly gatewayDenied: boolean;
  /** Downstream classification. Meaningful only when agentKeyValid. */
  readonly downstreamStatus: DownstreamStatus;
  /** One-sentence human diagnostic explaining the outcome. */
  readonly diagnostic: string;
}

interface ProbeOptions {
  readonly bearerToken: string;
  /** Verify the selected key, not another key whose secret was pasted. */
  readonly expectedKeyId: string;
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
  let body: unknown = null;
  try {
    response = await doFetch(url, {
      method: "GET",
      credentials: "omit",
      redirect: "manual",
      headers: {
        Authorization: `Bearer ${opts.bearerToken}`,
        "Content-Type": "application/json",
      },
      signal: controller.signal,
    });
    httpStatus = response.status;
    if (
      !response.redirected &&
      !response.headers.get("x-nyxid-agent-id")?.trim() &&
      [400, 402, 403, 404].includes(httpStatus) &&
      response.headers.get("content-type")?.split(";")[0]?.trim() ===
        "application/json"
    ) {
      body = await readGatewayEnvelope(response, controller.signal);
    }
  } catch {
    // Network error / abort / CORS — response stays null.
  } finally {
    window.clearTimeout(timer);
  }

  return classifyProbe(slug, response, httpStatus, body, opts.expectedKeyId);
}

async function readGatewayEnvelope(
  response: Response,
  signal: AbortSignal,
): Promise<unknown> {
  if (!response.body || signal.aborted) return null;
  const reader = response.body.getReader();
  const cancel = () => {
    void reader.cancel().catch(() => {});
  };
  signal.addEventListener("abort", cancel, { once: true });
  const decoder = new TextDecoder();
  let length = 0;
  let text = "";
  try {
    while (!signal.aborted) {
      const { done, value } = await reader.read();
      if (done)
        return signal.aborted ? null : JSON.parse(text + decoder.decode());
      length += value.byteLength;
      if (length > MAX_GATEWAY_ENVELOPE_BYTES) return null;
      text += decoder.decode(value, { stream: true });
    }
  } catch {
    // Invalid, interrupted or oversized envelopes remain an ordinary HTTP error.
  } finally {
    signal.removeEventListener("abort", cancel);
    cancel();
    reader.releaseLock();
  }
  return null;
}

/**
 * NyxID's error envelope, not a downstream's own `insufficient_credits`:
 * the platform always pairs the symbol with its numeric code.
 */
function isNyxidCreditsEnvelope(httpStatus: number, body: unknown): boolean {
  return (
    isInsufficientCreditsHttp(httpStatus, body) &&
    (body as { error_code?: unknown }).error_code === 11300
  );
}

/**
 * Pure classifier — exported for tests to exercise the header + status
 * logic without setting up a fetch mock every time.
 */
export function classifyProbe(
  slug: string,
  response: Response | null,
  httpStatus: number | null,
  body: unknown = null,
  expectedKeyId?: string,
): ProbeOutcome {
  if (!response || httpStatus === null) {
    return {
      httpStatus: null,
      reachedNyxid: false,
      agentKeyValid: false,
      gatewayDenied: false,
      downstreamStatus: "unexpected",
      diagnostic: translateUi(
        "The probe timed out, was blocked by the browser, or lost network. Retry, or check the browser devtools network tab.",
      ),
    };
  }

  // Guard against blank header values — a misconfigured reverse proxy
  // (or a bug in the axum handler) can send `X-NyxID-Agent-Id:` with
  // no value; `Headers.has()` returns true for that. We want the
  // truth signal to be the actual agent id string, so require length.
  // Header lookup is case-insensitive per WHATWG fetch spec.
  if (
    !response.headers.get("x-nyxid-agent-id")?.trim() &&
    isNyxidCreditsEnvelope(httpStatus, body)
  ) {
    // The gate may refuse before scope checks, so this proves nothing about
    // the key's reach; report it as not verified rather than as valid.
    return {
      httpStatus,
      reachedNyxid: true,
      agentKeyValid: false,
      gatewayDenied: false,
      downstreamStatus: "credits_exhausted",
      diagnostic: translateUi(
        "The account this key bills to is out of credits, so NyxID didn't run the test request. Test again once credits are added.",
      ),
    };
  }

  const agentIdHeader = response.headers.get("x-nyxid-agent-id");
  const agentKeyValid =
    !response.redirected &&
    typeof agentIdHeader === "string" &&
    agentIdHeader.trim().length > 0 &&
    (expectedKeyId === undefined || agentIdHeader === expectedKeyId);
  const reachedNyxid = true;

  if (!agentKeyValid) {
    return {
      httpStatus,
      reachedNyxid,
      agentKeyValid: false,
      gatewayDenied:
        !response.redirected &&
        !agentIdHeader?.trim() &&
        (isGatewayEnvelope(httpStatus, body, 403, "forbidden", 1002) ||
          isGatewayEnvelope(
            httpStatus,
            body,
            403,
            "api_key_scope_forbidden",
            9000,
          ) ||
          isGatewayEnvelope(httpStatus, body, 404, "not_found", 1003)),
      downstreamStatus: "unexpected",
      diagnostic:
        agentIdHeader?.trim() &&
        expectedKeyId !== undefined &&
        agentIdHeader !== expectedKeyId
          ? translateUi(
              "The response does not match the selected Agent Key. Check that you are using this key's full secret, then test again. You can also open the matching Agent Key's detail page.",
            )
          : isPendingAuthentication(httpStatus, body)
            ? translateUi(
                "This service is waiting for authentication. Open its detail page in AI Services, choose Continue authentication, and finish signing in with the provider. Then return here and test again.",
              )
            : diagnoseNyxidRejection(httpStatus, slug),
    };
  }

  const downstreamStatus = classifyDownstream(httpStatus);
  return {
    httpStatus,
    reachedNyxid,
    agentKeyValid: true,
    gatewayDenied: false,
    downstreamStatus,
    diagnostic: diagnoseDownstream(downstreamStatus, httpStatus),
  };
}

function isGatewayEnvelope(
  status: number,
  body: unknown,
  expectedStatus: number,
  error: string,
  errorCode: number,
): boolean {
  if (status !== expectedStatus || !body || typeof body !== "object")
    return false;
  const envelope = body as Record<string, unknown>;
  return envelope.error === error && envelope.error_code === errorCode;
}

function isPendingAuthentication(status: number, body: unknown): boolean {
  return (
    isGatewayEnvelope(status, body, 400, "bad_request", 1000) &&
    (body as Record<string, unknown>).message ===
      "Bad request: API key is pending_auth"
  );
}

/** Key/scope evidence alone cannot complete the first-call checklist. */
export function proxyCallSucceeded(outcome: ProbeOutcome): boolean {
  return (
    outcome.agentKeyValid &&
    outcome.httpStatus !== null &&
    outcome.httpStatus >= 200 &&
    outcome.httpStatus < 300 &&
    outcome.downstreamStatus === "ok"
  );
}

function classifyDownstream(status: number): DownstreamStatus {
  if (status >= 200 && status < 300) return "ok";
  if (status === 401 || status === 403) return "auth_rejected";
  if (status === 404) return "not_found";
  if (status >= 500) return "server_error";
  return "unexpected";
}

function diagnoseNyxidRejection(status: number, slug: string): string {
  if (status === 401) {
    return translateUi(
      "NyxID rejected the Agent Key. The key may be revoked, expired, or missing the `proxy` scope. Rotate the key and try again.",
    );
  }
  if (status === 403) {
    return translateUi(
      "NyxID rejected the request as out-of-scope. This Agent Key isn't allowed to call `{{slug}}` — add the service to the key's allowlist, or use a key with `allow_all_services`.",
      { slug },
    );
  }
  if (status === 404) {
    return translateUi(
      "NyxID doesn't know the slug `{{slug}}`. Was the service deleted, or is the slug spelled differently in your setup?",
      { slug },
    );
  }
  return translateUi(
    "Unexpected NyxID response (HTTP {{status}}). This shouldn't happen — check the browser devtools network tab for the response body.",
    { status },
  );
}

function diagnoseDownstream(
  status: DownstreamStatus,
  httpStatus: number,
): string {
  switch (status) {
    case "ok":
      return translateUi(
        "End-to-end verified (HTTP {{status}}). NyxID accepted your Agent Key and the downstream returned success. Use this Agent Key with the service's NyxID proxy URL in your AI tool.",
        { status: httpStatus },
      );
    case "auth_rejected":
      return translateUi(
        "Your Agent Key works (proxied through NyxID). The downstream rejected the stored credential (HTTP {{status}}) — reconnect the service, or update the credential from /keys.",
        { status: httpStatus },
      );
    case "not_found":
      return translateUi(
        "Your Agent Key works (proxied through NyxID). The downstream returned HTTP {{status}} on the probe path. This does not mean the key is broken, but a successful service call is still unverified. Check the service endpoint and test again.",
        { status: httpStatus },
      );
    case "server_error":
      return translateUi(
        "Your Agent Key works (proxied through NyxID). The downstream is having issues (HTTP {{status}}). Retry in a minute.",
        { status: httpStatus },
      );
    case "unexpected":
    case "credits_exhausted":
      return translateUi(
        "Your Agent Key works (proxied through NyxID). The downstream returned HTTP {{status}} — inspect the request in your AI tool to confirm the intended call succeeds.",
        { status: httpStatus },
      );
  }
}

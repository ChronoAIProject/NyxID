import { describe, expect, it, vi } from "vitest";
import {
  classifyProbe,
  isKnownUntestable,
  isTestable,
  PROBE_REGISTRY,
  probeAgentKey,
  probePathForSlug,
  recipeForSlug,
} from "./proxy-probe";

/** Build a Response whose headers include X-NyxID-Agent-Id. */
function nyxidResponse(status: number, agentId = "ag-123"): Response {
  return new Response("{}", {
    status,
    headers: { "x-nyxid-agent-id": agentId },
  });
}

/** Build a Response without the agent-id header (NyxID rejection). */
function rejectionResponse(status: number): Response {
  return new Response("{}", { status });
}

/**
 * Build a Response with X-NyxID-Agent-Id set to an EMPTY string.
 * A misconfigured reverse proxy (or an axum bug) can send this — a
 * naive `.has()` check would return true and mis-classify the probe
 * as agent-key-valid. Regression fence for GLM finding #2.
 */
function emptyHeaderResponse(status: number): Response {
  return new Response("{}", {
    status,
    headers: { "x-nyxid-agent-id": "" },
  });
}

describe("probePathForSlug — registry uses seeded service_slug forms", () => {
  // Registry keys match the backend's `service_slug` (with the
  // `llm-` / `api-` / `aws-` prefix baked in). If these tests break
  // with a "returns ''" failure, the prefix is missing from the
  // registry entry — production runtime passes prefixed slugs.
  it("returns `models` (bare) for llm-* OpenAI-compatible providers — base_urls already include the version segment", () => {
    // Kimi 2026-07-01: recipe paths must be RELATIVE to the seeded
    // base_url (which for openai/anthropic/mistral/deepseek ends in
    // `/v1`, google-ai `/v1beta`, cohere `/v2`). Duplicating the
    // version segment produced `.../v1/v1/models` → silent 404 for
    // every LLM probe.
    expect(probePathForSlug("llm-openai")).toBe("models");
    expect(probePathForSlug("llm-anthropic")).toBe("models");
    expect(probePathForSlug("llm-deepseek")).toBe("models");
    expect(probePathForSlug("llm-mistral")).toBe("models");
    expect(probePathForSlug("llm-cohere")).toBe("models");
    expect(probePathForSlug("llm-google-ai")).toBe("models");
  });

  it("llm-openrouter probes the credential-validating `key` endpoint, not `models`", () => {
    // OpenRouter's /models is public (200 without auth) and would
    // green-light an invalid credential; GET {base}/key 401s on a bad
    // key, so it is the only high-confidence probe for this provider.
    expect(probePathForSlug("llm-openrouter")).toBe("key");
    expect(isTestable("llm-openrouter")).toBe(true);
    // Repeat-connect suffix resolves to the same recipe.
    expect(probePathForSlug("llm-openrouter-2")).toBe("key");
  });

  it("returns provider-specific paths (relative to base_url) for api-* known providers", () => {
    // Bare bases (no version in base_url) → recipe carries the path
    expect(probePathForSlug("api-github")).toBe("user");
    expect(probePathForSlug("api-github-pat")).toBe("user");
    expect(probePathForSlug("api-telegram-bot")).toBe("getMe");
    expect(probePathForSlug("api-google")).toBe("oauth2/v1/userinfo");
    // Bases with version/api segment → recipe drops the duplicated prefix
    expect(probePathForSlug("api-discord-bot")).toBe("users/@me");
    expect(probePathForSlug("api-discord")).toBe("users/@me");
    expect(probePathForSlug("api-slack-bot")).toBe("auth.test");
    expect(probePathForSlug("api-slack")).toBe("auth.test");
    expect(probePathForSlug("api-spotify")).toBe("me");
    expect(probePathForSlug("api-microsoft")).toBe("me");
    expect(probePathForSlug("api-twitter")).toBe("users/me");
    expect(probePathForSlug("api-twitch")).toBe("users");
    expect(probePathForSlug("api-facebook")).toBe("me");
    expect(probePathForSlug("api-lark")).toBe("authen/v1/user_info");
    expect(probePathForSlug("api-feishu")).toBe("authen/v1/user_info");
  });

  it("returns '' for explicitly untestable seeded slugs", () => {
    // Registered as `null` — no probe endpoint we can rely on.
    expect(probePathForSlug("llm-openai-codex")).toBe("");
    expect(probePathForSlug("llm-openclaw")).toBe("");
    expect(probePathForSlug("api-lark-bot")).toBe("");
    expect(probePathForSlug("api-feishu-bot")).toBe("");
    expect(probePathForSlug("api-firecrawl")).toBe("");
    expect(probePathForSlug("api-tiktok")).toBe("");
    expect(probePathForSlug("aws-cost-explorer")).toBe("");
    expect(probePathForSlug("api-google-cloud")).toBe("");
  });

  it("strips repeat-connect `-N` suffix (llm-openai-2 → llm-openai)", () => {
    expect(probePathForSlug("llm-openai-2")).toBe("models");
    expect(probePathForSlug("llm-anthropic-99")).toBe("models");
    expect(probePathForSlug("api-github-3")).toBe("user");
    expect(probePathForSlug("api-telegram-bot-7")).toBe("getMe");
    // Untestable base survives the suffix strip
    expect(probePathForSlug("llm-openai-codex-7")).toBe("");
    expect(probePathForSlug("api-firecrawl-2")).toBe("");
  });

  it("returns '' for unregistered custom slugs (no more openai-family fallback)", () => {
    // The old code fell through to an openai-family regex for slugs
    // that "looked openai-shaped". That created misleading probes on
    // custom endpoints named `perplexity` or `groq`. Now we require
    // an explicit registry entry.
    expect(probePathForSlug("perplexity")).toBe("");
    expect(probePathForSlug("my-custom-groq-relay")).toBe("");
    expect(probePathForSlug("acme-internal-api")).toBe("");
  });
});

describe("PROBE_REGISTRY — table-driven coverage (typo trap)", () => {
  // Walking the entire PROBE_REGISTRY catches typos in ANY entry —
  // e.g. writing `getMe/` instead of `getMe` for telegram-bot would
  // pass hand-picked slug tests but fail this walk. Also catches
  // accidental removal of a registry entry (test count changes).
  it("every registered testable provider has an exact probePathForSlug match", () => {
    const testable = Object.entries(PROBE_REGISTRY).filter(
      ([, recipe]) => recipe !== null,
    );
    // Sanity check — if the registry shrinks unexpectedly, we notice.
    // 19 testable seeded slugs today (6 llm-* + 13 api-*).
    expect(testable.length).toBeGreaterThanOrEqual(19);

    for (const [slug, recipe] of testable) {
      if (!recipe) continue; // narrowing (already filtered)
      expect(
        probePathForSlug(slug),
        `probePathForSlug("${slug}") should return the recipe path`,
      ).toBe(recipe.path);
      expect(isTestable(slug)).toBe(true);
      expect(isKnownUntestable(slug)).toBe(false);
    }
  });

  it("every registered untestable provider returns null recipe + '' probe path", () => {
    const untestable = Object.entries(PROBE_REGISTRY).filter(
      ([, recipe]) => recipe === null,
    );
    // 8 untestable seeded slugs (llm-openai-codex, llm-openclaw,
    // api-firecrawl, api-tiktok, api-lark-bot, api-feishu-bot,
    // aws-cost-explorer, api-google-cloud).
    expect(untestable.length).toBeGreaterThanOrEqual(8);

    for (const [slug] of untestable) {
      expect(recipeForSlug(slug)).toBeNull();
      expect(probePathForSlug(slug)).toBe("");
      expect(isKnownUntestable(slug)).toBe(true);
      expect(isTestable(slug)).toBe(false);
    }
  });
});

describe("recipeForSlug — suffix strip edge cases", () => {
  it("does NOT strip letter-only suffixes (llm-openai-abc → miss)", () => {
    expect(recipeForSlug("llm-openai-abc")).toBeUndefined();
    expect(probePathForSlug("llm-openai-abc")).toBe("");
    expect(isTestable("llm-openai-abc")).toBe(false);
  });

  it("strips only the trailing digit-run (llm-openai-2-3 → llm-openai-2 → llm-openai)", () => {
    // Regex is greedy on trailing digits only; each recipeForSlug
    // call runs the strip once. `-2-3` becomes `-2`, then a second
    // resolution (would need external loop) — the internal single
    // pass strips just `-3` and lands on `llm-openai-2`, which IS in
    // the registry key form only for the current wizard shape. We
    // pin the CURRENT single-pass semantics: `-2-3` resolves to
    // `llm-openai-2` which is not in registry → undefined.
    // If NyxID's wizard ever emits deeper suffixes, extend the loop
    // in recipeForSlug rather than compensating in tests.
    expect(recipeForSlug("llm-openai-2-3")).toBeUndefined();
    expect(probePathForSlug("llm-openai-2-3")).toBe("");
  });

  it("does NOT strip digit+letter tails (llm-openai-2abc → miss)", () => {
    expect(recipeForSlug("llm-openai-2abc")).toBeUndefined();
    expect(probePathForSlug("llm-openai-2abc")).toBe("");
    expect(isTestable("llm-openai-2abc")).toBe(false);
  });

  it("empty and whitespace slugs are unknown, not untestable", () => {
    expect(recipeForSlug("")).toBeUndefined();
    expect(isTestable("")).toBe(false);
    expect(isKnownUntestable("")).toBe(false); // undefined, not null
  });
});

describe("recipeForSlug + isKnownUntestable + isTestable — semantic truth table", () => {
  it("recipeForSlug returns a recipe object for registered testable seeded slugs", () => {
    expect(recipeForSlug("llm-openai")).toEqual({ path: "models" });
    expect(recipeForSlug("api-telegram-bot")).toEqual({ path: "getMe" });
    expect(recipeForSlug("api-github-pat")).toEqual({ path: "user" });
  });

  it("recipeForSlug returns null (not undefined) for registered UNTESTABLE slugs", () => {
    // null vs undefined distinguishes "we've seen this and can't
    // probe it" from "we've never heard of this slug".
    expect(recipeForSlug("llm-openai-codex")).toBeNull();
    expect(recipeForSlug("llm-openclaw")).toBeNull();
    expect(recipeForSlug("api-firecrawl")).toBeNull();
  });

  it("recipeForSlug returns undefined for unknown/custom slugs", () => {
    expect(recipeForSlug("acme-internal-api")).toBeUndefined();
    expect(recipeForSlug("perplexity")).toBeUndefined();
  });

  it("isTestable is TRUE only when an explicit ProbeRecipe exists (high confidence)", () => {
    // Registered testable → true
    expect(isTestable("llm-openai")).toBe(true);
    expect(isTestable("llm-anthropic")).toBe(true);
    expect(isTestable("api-github")).toBe(true);
    expect(isTestable("api-telegram-bot")).toBe(true);
    expect(isTestable("llm-openai-2")).toBe(true); // suffix-strip
    expect(isTestable("api-github-pat-3")).toBe(true); // suffix-strip

    // Registered null → false
    expect(isTestable("llm-openai-codex")).toBe(false);
    expect(isTestable("llm-openai-codex-7")).toBe(false);
    expect(isTestable("llm-openclaw")).toBe(false);
    expect(isTestable("api-firecrawl")).toBe(false);
    expect(isTestable("api-lark-bot")).toBe(false);

    // Unregistered (custom endpoints) → false. The high-confidence
    // rule requires an EXPLICIT recipe — no more openai-family
    // regex fallback since it produced misleading probes on custom
    // endpoints that happened to include an LLM name.
    expect(isTestable("acme-internal-api")).toBe(false);
    expect(isTestable("my-custom-endpoint")).toBe(false);
    expect(isTestable("perplexity")).toBe(false); // unregistered
  });

  it("isTestable handles undefined/empty defensively", () => {
    expect(isTestable("")).toBe(false);
    expect(isTestable(undefined as unknown as string)).toBe(false);
  });

  it("isKnownUntestable is TRUE only for slugs explicitly registered as null", () => {
    // The 8 slugs registered as null across the whole catalog seed.
    expect(isKnownUntestable("llm-openai-codex")).toBe(true);
    expect(isKnownUntestable("llm-openai-codex-7")).toBe(true);
    expect(isKnownUntestable("llm-openclaw")).toBe(true);
    expect(isKnownUntestable("api-lark-bot")).toBe(true);
    expect(isKnownUntestable("api-feishu-bot")).toBe(true);
    expect(isKnownUntestable("api-firecrawl")).toBe(true);
    expect(isKnownUntestable("api-tiktok")).toBe(true);
    expect(isKnownUntestable("aws-cost-explorer")).toBe(true);
    expect(isKnownUntestable("api-google-cloud")).toBe(true);

    // Registered testable → false
    expect(isKnownUntestable("llm-openai")).toBe(false);
    expect(isKnownUntestable("api-telegram-bot")).toBe(false);

    // Unknown/custom → false (undefined recipe, not null)
    expect(isKnownUntestable("acme-internal-api")).toBe(false);
    expect(isKnownUntestable("perplexity")).toBe(false);
  });
});

describe("classifyProbe — network / timeout", () => {
  it("null response → not-reached-NyxID, not-valid, unexpected downstream, actionable diagnostic", () => {
    const outcome = classifyProbe("openai", null, null);
    expect(outcome.reachedNyxid).toBe(false);
    expect(outcome.agentKeyValid).toBe(false);
    expect(outcome.diagnostic).toMatch(/timed out|blocked|network/i);
  });
});

describe("classifyProbe — NyxID rejects (no X-NyxID-Agent-Id header)", () => {
  it("401 → agent key invalid, diagnostic mentions revoked/scope/rotate", () => {
    const outcome = classifyProbe("openai", rejectionResponse(401), 401);
    expect(outcome.agentKeyValid).toBe(false);
    expect(outcome.httpStatus).toBe(401);
    // Pin the full outcome shape — GLM finding #3: silent-pass trap
    // if downstreamStatus drifts. All NyxID-layer rejections must
    // land in `unexpected` because downstream never got hit.
    expect(outcome.downstreamStatus).toBe("unexpected");
    expect(outcome.diagnostic).toMatch(/rejected the Agent Key/i);
  });

  it("403 → scope failure, diagnostic names the slug that was out-of-scope", () => {
    const outcome = classifyProbe("openai", rejectionResponse(403), 403);
    expect(outcome.agentKeyValid).toBe(false);
    expect(outcome.downstreamStatus).toBe("unexpected");
    expect(outcome.diagnostic).toMatch(/out-of-scope/i);
    expect(outcome.diagnostic).toMatch(/`openai`/);
  });

  it("404 (no header) → NyxID doesn't know the slug", () => {
    const outcome = classifyProbe("mystery", rejectionResponse(404), 404);
    expect(outcome.agentKeyValid).toBe(false);
    expect(outcome.downstreamStatus).toBe("unexpected");
    expect(outcome.diagnostic).toMatch(/doesn't know the slug/i);
    expect(outcome.diagnostic).toMatch(/`mystery`/);
  });

  it("REGRESSION (GLM #2): X-NyxID-Agent-Id header present but EMPTY string → agent key invalid", () => {
    // A misconfigured reverse proxy can send the header without a
    // value. `.has()` returns true; only the value's non-empty length
    // proves NyxID stamped a real agent id. If this ever breaks the
    // agent-id header becomes trivially forgeable by a rogue proxy.
    const outcome = classifyProbe("openai", emptyHeaderResponse(200), 200);
    expect(outcome.agentKeyValid).toBe(false);
    // With no valid header we can't trust ANY downstream signal, so
    // this must NOT flow into the "auth_rejected"/"ok" downstream
    // classification path.
    expect(outcome.downstreamStatus).toBe("unexpected");
  });

  it("uppercase X-NyxID-Agent-Id header is accepted (WHATWG case-insensitivity)", () => {
    // Response header lookups MUST be case-insensitive per fetch
    // spec. Some proxies (nginx, older ALB) uppercase the header.
    const res = new Response("{}", {
      status: 200,
      headers: { "X-NyxID-Agent-Id": "ag-123" },
    });
    const outcome = classifyProbe("openai", res, 200);
    expect(outcome.agentKeyValid).toBe(true);
    expect(outcome.downstreamStatus).toBe("ok");
  });
});

describe("classifyProbe — NyxID accepted (X-NyxID-Agent-Id present)", () => {
  it("200 → downstream ok, end-to-end verified", () => {
    const outcome = classifyProbe("openai", nyxidResponse(200), 200);
    expect(outcome.agentKeyValid).toBe(true);
    expect(outcome.downstreamStatus).toBe("ok");
    expect(outcome.diagnostic).toMatch(/End-to-end verified/i);
  });

  it("403 with X-NyxID-Agent-Id → agent key valid, downstream auth_rejected (not out-of-scope)", () => {
    // Kimi finding — this case wasn't covered. classifyDownstream
    // maps 401 AND 403 to `auth_rejected`. Pins both.
    const outcome = classifyProbe("openai", nyxidResponse(403), 403);
    expect(outcome.agentKeyValid).toBe(true);
    expect(outcome.downstreamStatus).toBe("auth_rejected");
    expect(outcome.diagnostic).toMatch(/Agent Key works/i);
    expect(outcome.diagnostic).toMatch(/rejected the stored credential/i);
  });

  it("401 with X-NyxID-Agent-Id → agent key VALID, downstream rejected the stored credential", () => {
    // This is the critical case the old probe got wrong: a 401 with
    // the agent-id header means the Agent Key is fine, only the
    // stored upstream credential is broken. The verdict must be
    // agentKeyValid=true so the panel + copy flow continue.
    const outcome = classifyProbe("openai", nyxidResponse(401), 401);
    expect(outcome.agentKeyValid).toBe(true);
    expect(outcome.downstreamStatus).toBe("auth_rejected");
    expect(outcome.diagnostic).toMatch(/Agent Key works/i);
    expect(outcome.diagnostic).toMatch(/rejected the stored credential/i);
  });

  it("404 with X-NyxID-Agent-Id → agent key VALID, downstream just doesn't have the probe path", () => {
    // The other critical case — services like telegram-bot, discord-bot,
    // openclaw etc. that 404 the probe path even when everything is
    // configured. Verdict must be agentKeyValid=true.
    const outcome = classifyProbe("telegram-bot", nyxidResponse(404), 404);
    expect(outcome.agentKeyValid).toBe(true);
    expect(outcome.downstreamStatus).toBe("not_found");
    expect(outcome.diagnostic).toMatch(/does not mean the key is broken/i);
  });

  it("502 → downstream server error, agent key still valid", () => {
    const outcome = classifyProbe("openai", nyxidResponse(502), 502);
    expect(outcome.agentKeyValid).toBe(true);
    expect(outcome.downstreamStatus).toBe("server_error");
    expect(outcome.diagnostic).toMatch(/Retry in a minute/i);
  });

  it("418 (weird) → downstream unexpected, agent key still valid", () => {
    const outcome = classifyProbe("openai", nyxidResponse(418), 418);
    expect(outcome.agentKeyValid).toBe(true);
    expect(outcome.downstreamStatus).toBe("unexpected");
    expect(outcome.diagnostic).toMatch(/inspect the request/i);
  });
});

describe("probeAgentKey — fetch integration", () => {
  it("uses per-slug probe path when none given, encodes slug in URL, sends Bearer + Content-Type", async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(nyxidResponse(200));

    await probeAgentKey("llm-openai", {
      bearerToken: "nyxid_ag_test",
      fetchImpl: fetchMock,
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const firstCall = fetchMock.mock.calls[0];
    if (!firstCall) throw new Error("fetch was not called");
    const [url, init] = firstCall;
    expect(String(url)).toBe("/api/v1/proxy/s/llm-openai/models");
    expect(init).toMatchObject({
      method: "GET",
      credentials: "omit",
      headers: expect.objectContaining({
        Authorization: "Bearer nyxid_ag_test",
        "Content-Type": "application/json",
      }),
    });
  });

  it("respects probePath override", async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(nyxidResponse(200));

    await probeAgentKey("custom", {
      bearerToken: "tok",
      probePath: "custom/path",
      fetchImpl: fetchMock,
    });

    const call = fetchMock.mock.calls[0];
    if (!call) throw new Error("fetch was not called");
    expect(String(call[0])).toBe("/api/v1/proxy/s/custom/custom/path");
  });

  it("returns a network-error outcome when fetch throws", async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockRejectedValue(new TypeError("network down"));

    const outcome = await probeAgentKey("openai", {
      bearerToken: "tok",
      fetchImpl: fetchMock,
    });

    expect(outcome.reachedNyxid).toBe(false);
    expect(outcome.agentKeyValid).toBe(false);
    expect(outcome.httpStatus).toBeNull();
  });

  it("forwards the AbortController signal to fetch (regression fence for timeout wiring)", async () => {
    // GLM finding #1 + Kimi Medium — if the abort/signal wiring is
    // ever stripped, this test fails immediately. Otherwise the
    // production code could time out silently and no test would catch it.
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(nyxidResponse(200));

    await probeAgentKey("openai", {
      bearerToken: "tok",
      fetchImpl: fetchMock,
    });

    const call = fetchMock.mock.calls[0];
    if (!call) throw new Error("fetch was not called");
    const [, init] = call;
    expect(init?.signal).toBeInstanceOf(AbortSignal);
  });

  it("timeout: a fetch that never resolves resolves the outcome as network-error via AbortController", async () => {
    // Simulates a hanging downstream. `probeAgentKey` sets a
    // setTimeout that aborts the signal after `timeoutMs`; the fetch
    // impl must resolve/reject in response to abort. We wire the
    // fetch mock to reject IF the signal aborts (matching real fetch
    // behavior), and set a tiny timeout so the test doesn't stall.
    const fetchMock = vi.fn<typeof fetch>().mockImplementation(
      (_url, init) =>
        new Promise((_resolve, reject) => {
          const signal = (init as RequestInit | undefined)?.signal;
          if (signal) {
            signal.addEventListener("abort", () => {
              reject(new DOMException("aborted", "AbortError"));
            });
          }
        }),
    );

    const outcome = await probeAgentKey("openai", {
      bearerToken: "tok",
      fetchImpl: fetchMock,
      timeoutMs: 10,
    });

    expect(outcome.httpStatus).toBeNull();
    expect(outcome.reachedNyxid).toBe(false);
    expect(outcome.agentKeyValid).toBe(false);
  });
});

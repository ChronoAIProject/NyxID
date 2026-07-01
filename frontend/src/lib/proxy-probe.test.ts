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

describe("probePathForSlug — registry-first", () => {
  it("returns v1/models for openai-family slugs from registry", () => {
    expect(probePathForSlug("openai")).toBe("v1/models");
    expect(probePathForSlug("anthropic")).toBe("v1/models");
    expect(probePathForSlug("deepseek")).toBe("v1/models");
    expect(probePathForSlug("mistral")).toBe("v1/models");
    expect(probePathForSlug("cohere")).toBe("v1/models");
    expect(probePathForSlug("google-ai")).toBe("v1/models");
  });

  it("returns provider-specific paths for known non-LLM providers", () => {
    expect(probePathForSlug("github")).toBe("user");
    expect(probePathForSlug("github-pat")).toBe("user");
    expect(probePathForSlug("telegram-bot")).toBe("getMe");
    expect(probePathForSlug("discord-bot")).toBe("v10/users/@me");
    expect(probePathForSlug("slack-bot")).toBe("api/auth.test");
    expect(probePathForSlug("spotify")).toBe("v1/me");
    expect(probePathForSlug("google")).toBe("oauth2/v1/userinfo");
  });

  it("returns '' for explicitly untestable providers (Codex, OpenClaw, Lark/Feishu bot)", () => {
    // These slugs are in the registry as `null` — no cheap probe exists.
    // probePathForSlug returns "" so a caller that ignores untestable
    // still hits the root and gets the header-based fallback signal.
    expect(probePathForSlug("openai-codex")).toBe("");
    expect(probePathForSlug("openclaw")).toBe("");
    expect(probePathForSlug("lark-bot")).toBe("");
    expect(probePathForSlug("feishu-bot")).toBe("");
    expect(probePathForSlug("firecrawl")).toBe("");
  });

  it("strips repeat-connect suffixes (`openai-2`, `github-3`)", () => {
    expect(probePathForSlug("openai-2")).toBe("v1/models");
    expect(probePathForSlug("openai-99")).toBe("v1/models");
    expect(probePathForSlug("github-3")).toBe("user");
    expect(probePathForSlug("telegram-bot-7")).toBe("getMe");
    // Untestable base survives the suffix strip
    expect(probePathForSlug("openai-codex-7")).toBe("");
  });

  it("strips `llm-` prefixes from wizard/catalog naming (`llm-openai-codex`)", () => {
    expect(probePathForSlug("llm-openai-codex")).toBe("");
    expect(probePathForSlug("llm-openai-codex-7")).toBe("");
    expect(probePathForSlug("llm-anthropic")).toBe("v1/models");
  });

  it("falls back to openai-family regex for unregistered custom slugs", () => {
    // `perplexity` isn't in the registry but matches the family regex.
    expect(probePathForSlug("perplexity")).toBe("v1/models");
    expect(probePathForSlug("my-custom-groq-relay")).toBe("v1/models");
  });

  it("returns '' for slugs with no known cheap probe endpoint", () => {
    expect(probePathForSlug("some-random-custom-thing")).toBe("");
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
    expect(untestable.length).toBeGreaterThanOrEqual(5);

    for (const [slug] of untestable) {
      expect(recipeForSlug(slug)).toBeNull();
      expect(probePathForSlug(slug)).toBe("");
      expect(isKnownUntestable(slug)).toBe(true);
      expect(isTestable(slug)).toBe(false);
    }
  });
});

describe("recipeForSlug — suffix/prefix strip edge cases", () => {
  // Kimi + GLM both flagged that suffix stripping had thin coverage.
  // These tests pin the exact behavior — the regex is
  // `.replace(/-\d+$/, "")` (last digit-run only, digit-only).
  it("does NOT strip letter-only suffixes (openai-abc → miss)", () => {
    // `openai-abc` shouldn't be treated as `openai` — someone named a
    // custom endpoint that way and we shouldn't guess.
    expect(recipeForSlug("openai-abc")).toBeUndefined();
    expect(probePathForSlug("openai-abc")).toBe("v1/models"); // regex fallback
    expect(isTestable("openai-abc")).toBe(false); // no explicit recipe
  });

  it("strips only the LAST digit-run for chained suffixes (openai-2-3 → openai-2)", () => {
    // Regex is greedy on the trailing digits only. `openai-2-3` strips
    // `-3`, leaves `openai-2`, which itself isn't in the registry so
    // one more strip attempt fires internally (via the base-no-suffix
    // path). Confirm the final resolution lands on `openai`.
    expect(probePathForSlug("openai-2-3")).toBe("v1/models");
  });

  it("does NOT strip digit+letter tails (openai-2abc → miss, then regex fallback)", () => {
    // The regex requires the suffix to be pure digits.
    expect(recipeForSlug("openai-2abc")).toBeUndefined();
    // Regex fallback catches openai-family shape.
    expect(probePathForSlug("openai-2abc")).toBe("v1/models");
    expect(isTestable("openai-2abc")).toBe(false);
  });

  it("handles the compound `llm-<slug>-N` shape (llm-anthropic-2 → v1/models)", () => {
    // NyxID's wizard emits `llm-anthropic-2` for the second anthropic
    // connect. Suffix strip runs first (→ `llm-anthropic`), then llm-
    // prefix strip (→ `anthropic`).
    expect(probePathForSlug("llm-anthropic-2")).toBe("v1/models");
    expect(recipeForSlug("llm-anthropic-2")).toEqual({ path: "v1/models" });
  });

  it("empty and whitespace slugs are unknown, not untestable", () => {
    expect(recipeForSlug("")).toBeUndefined();
    expect(isTestable("")).toBe(false);
    expect(isKnownUntestable("")).toBe(false); // undefined, not null
  });
});

describe("recipeForSlug + isKnownUntestable", () => {
  it("recipeForSlug returns a recipe object for registered testable providers", () => {
    expect(recipeForSlug("openai")).toEqual({ path: "v1/models" });
    expect(recipeForSlug("telegram-bot")).toEqual({ path: "getMe" });
  });

  it("recipeForSlug returns null (not undefined) for registered UNTESTABLE providers", () => {
    // The null-vs-undefined distinction matters for isKnownUntestable.
    expect(recipeForSlug("openai-codex")).toBeNull();
    expect(recipeForSlug("openclaw")).toBeNull();
  });

  it("recipeForSlug returns undefined for unknown/custom slugs", () => {
    expect(recipeForSlug("acme-internal-api")).toBeUndefined();
  });

  it("isTestable is TRUE only when we have an explicit ProbeRecipe (high confidence)", () => {
    // Registered testable → true
    expect(isTestable("openai")).toBe(true);
    expect(isTestable("anthropic")).toBe(true);
    expect(isTestable("github")).toBe(true);
    expect(isTestable("telegram-bot")).toBe(true);
    expect(isTestable("openai-2")).toBe(true); // suffix-strip
    expect(isTestable("llm-anthropic")).toBe(true); // llm-prefix strip

    // Registered null → false
    expect(isTestable("openai-codex")).toBe(false);
    expect(isTestable("llm-openai-codex-7")).toBe(false);
    expect(isTestable("openclaw")).toBe(false);
    expect(isTestable("lark-bot")).toBe(false);

    // Unregistered (custom endpoints) → false. The high-confidence
    // rule requires an EXPLICIT recipe — falling through to the
    // openai regex or root probe isn't enough to offer a Test
    // button. Better to hide it than mislead the user.
    expect(isTestable("acme-internal-api")).toBe(false);
    expect(isTestable("my-custom-endpoint")).toBe(false);
    // Even openai-family regex matches don't count if they're not
    // registered explicitly — `perplexity` looks openai-shaped but
    // has no seed recipe, so no Test button.
    expect(isTestable("perplexity")).toBe(false);
    expect(isTestable("groq")).toBe(false);
  });

  it("isTestable handles undefined/empty defensively", () => {
    expect(isTestable("")).toBe(false);
    expect(isTestable(undefined as unknown as string)).toBe(false);
  });

  it("isKnownUntestable is TRUE only for slugs explicitly registered as null", () => {
    // Codex + OpenClaw + Lark/Feishu bot are the known-untestable set.
    expect(isKnownUntestable("openai-codex")).toBe(true);
    expect(isKnownUntestable("llm-openai-codex-7")).toBe(true);
    expect(isKnownUntestable("openclaw")).toBe(true);
    expect(isKnownUntestable("lark-bot")).toBe(true);
    expect(isKnownUntestable("feishu-bot")).toBe(true);
    expect(isKnownUntestable("firecrawl")).toBe(true);

    // Anything with a recipe → testable
    expect(isKnownUntestable("openai")).toBe(false);
    expect(isKnownUntestable("telegram-bot")).toBe(false);

    // Unknown/custom → assumed testable (falls through to root probe
    // with header-based classifier as the safety net)
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

    await probeAgentKey("openai", {
      bearerToken: "nyxid_ag_test",
      fetchImpl: fetchMock,
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const firstCall = fetchMock.mock.calls[0];
    if (!firstCall) throw new Error("fetch was not called");
    const [url, init] = firstCall;
    expect(String(url)).toBe("/api/v1/proxy/s/openai/v1/models");
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

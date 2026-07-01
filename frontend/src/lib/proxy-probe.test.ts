import { describe, expect, it, vi } from "vitest";
import {
  classifyProbe,
  probeAgentKey,
  probePathForSlug,
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

describe("probePathForSlug", () => {
  it("returns v1/models for openai-family slugs (openai, anthropic, deepseek, mistral, cohere)", () => {
    expect(probePathForSlug("openai")).toBe("v1/models");
    expect(probePathForSlug("openai-codex")).toBe("v1/models");
    expect(probePathForSlug("anthropic")).toBe("v1/models");
    expect(probePathForSlug("deepseek")).toBe("v1/models");
    expect(probePathForSlug("mistral")).toBe("v1/models");
    expect(probePathForSlug("cohere")).toBe("v1/models");
    // The seed doesn't have `google-ai-2` — regex should still catch it.
    expect(probePathForSlug("anthropic-2")).toBe("v1/models");
  });

  it("returns user for github + github-pat", () => {
    expect(probePathForSlug("github")).toBe("user");
    expect(probePathForSlug("github-pat")).toBe("user");
  });

  it("returns '' for slugs with no known cheap probe endpoint", () => {
    expect(probePathForSlug("telegram-bot")).toBe("");
    expect(probePathForSlug("discord-bot")).toBe("");
    expect(probePathForSlug("openclaw")).toBe("");
    expect(probePathForSlug("some-custom-thing")).toBe("");
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
    expect(outcome.diagnostic).toMatch(/rejected the Agent Key/i);
  });

  it("403 → scope failure, diagnostic names the slug that was out-of-scope", () => {
    const outcome = classifyProbe("openai", rejectionResponse(403), 403);
    expect(outcome.agentKeyValid).toBe(false);
    expect(outcome.diagnostic).toMatch(/out-of-scope/i);
    expect(outcome.diagnostic).toMatch(/`openai`/);
  });

  it("404 (no header) → NyxID doesn't know the slug", () => {
    const outcome = classifyProbe("mystery", rejectionResponse(404), 404);
    expect(outcome.agentKeyValid).toBe(false);
    expect(outcome.diagnostic).toMatch(/doesn't know the slug/i);
    expect(outcome.diagnostic).toMatch(/`mystery`/);
  });
});

describe("classifyProbe — NyxID accepted (X-NyxID-Agent-Id present)", () => {
  it("200 → downstream ok, end-to-end verified", () => {
    const outcome = classifyProbe("openai", nyxidResponse(200), 200);
    expect(outcome.agentKeyValid).toBe(true);
    expect(outcome.downstreamStatus).toBe("ok");
    expect(outcome.diagnostic).toMatch(/End-to-end verified/i);
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
});

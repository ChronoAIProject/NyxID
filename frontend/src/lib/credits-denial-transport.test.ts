import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { apiClient, ApiError } from "@/lib/api-client";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import { shouldRetryQuery } from "@/lib/query-retry";
import { useCreditsDenialStore } from "@/stores/credits-denial-store";

const denied = {
  error: "insufficient_credits",
  error_code: 11300,
  message: "Insufficient credits",
};
const response = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
const optIn = { key: "op:test:1", payer: "self" as const };

beforeEach(() => {
  transitionAssistantIdentity("person-a");
  useCreditsDenialStore.getState().reset();
});
afterEach(() => vi.unstubAllGlobals());

describe("apiClient credits opt-in", () => {
  it("notifies and still throws the original ApiError", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => response(402, denied)),
    );
    const error = await apiClient("/channel-relay/send", {
      method: "POST",
      creditsDenial: optIn,
    }).catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).status).toBe(402);
    expect(useCreditsDenialStore.getState().current).toMatchObject(optIn);
  });

  it("never notifies without the opt-in", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => response(402, denied)),
    );
    await expect(apiClient("/billing/usage")).rejects.toBeInstanceOf(ApiError);
    expect(useCreditsDenialStore.getState().current).toBeNull();
  });

  it("ignores a response that lands after the account switched", async () => {
    let resolve!: (value: Response) => void;
    vi.stubGlobal(
      "fetch",
      vi.fn(() => new Promise<Response>((done) => (resolve = done))),
    );
    const request = apiClient("/x", { method: "POST", creditsDenial: optIn });
    await vi.waitFor(() => expect(resolve).toBeTypeOf("function"));
    transitionAssistantIdentity("person-b");
    resolve(response(402, denied));
    await expect(request).rejects.toBeInstanceOf(ApiError);
    expect(useCreditsDenialStore.getState().current).toBeNull();
  });
});

describe("assistantHttp credits opt-in", () => {
  it("classifies the raw envelope, including a numeric-only 402", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => response(402, { error_code: 11300, message: "x" })),
    );
    await expect(
      assistantHttp("/assistant/chat", {
        method: "POST",
        creditsDenial: optIn,
      }),
    ).rejects.toBeInstanceOf(ApiError);
    expect(useCreditsDenialStore.getState().current?.key).toBe("op:test:1");
  });

  it("does not notify for another 402 symbol", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        response(402, { error: "wallet_suspended", error_code: 11302 }),
      ),
    );
    await expect(
      assistantHttp("/assistant/chat", {
        method: "POST",
        creditsDenial: optIn,
      }),
    ).rejects.toBeInstanceOf(ApiError);
    expect(useCreditsDenialStore.getState().current).toBeNull();
  });
});

describe("shouldRetryQuery", () => {
  it("does not retry a recognized denial or a 401", () => {
    expect(shouldRetryQuery(0, new ApiError(402, denied))).toBe(false);
    expect(shouldRetryQuery(0, { status: 401 })).toBe(false);
  });

  it("keeps retrying other failures up to three times", () => {
    const other = new ApiError(402, { ...denied, error: "wallet_suspended" });
    expect(shouldRetryQuery(0, other)).toBe(true);
    expect(shouldRetryQuery(3, new Error("boom"))).toBe(false);
  });
});

import { afterEach, describe, expect, it, vi } from "vitest";
import { startManagedOAuth, completeManagedOAuth } from "./use-channel-managed";
import { apiClient } from "@/lib/api-client";

vi.mock("@/lib/api-client", () => ({
  apiClient: vi.fn(),
  api: {},
  apiFetch: vi.fn(),
  ApiError: class extends Error {},
}));
afterEach(() => vi.clearAllMocks());
const connection = "11111111-1111-4111-8111-111111111111";
const signal = new AbortController().signal;
describe("managed OAuth transport", () => {
  it("starts a scoped connection with the caller's abort signal", async () => {
    vi.mocked(apiClient).mockResolvedValue({
      connection_id: connection,
      attempt_nonce: connection,
      authorization_url: "https://x.com/authorize",
    });
    await startManagedOAuth("x", " Support ", "org", signal);
    expect(apiClient).toHaveBeenCalledWith(
      "/channel-bots/managed-onboarding/x/start",
      {
        method: "POST",
        body: { label: "Support", target_org_id: "org" },
        signal,
      },
    );
  });
  it("completes and reconnects without returning OAuth tokens to the server", async () => {
    vi.mocked(apiClient).mockResolvedValue({ id: "bot", platform: "x" });
    await completeManagedOAuth(
      "x",
      { connection_id: connection, label: "Support" },
      signal,
    );
    expect(apiClient).toHaveBeenLastCalledWith(
      "/channel-bots/managed-onboarding/x/complete",
      {
        method: "POST",
        body: { connection_id: connection, label: "Support" },
        signal,
      },
    );
    await completeManagedOAuth(
      "x",
      { connection_id: connection, label: "Support" },
      signal,
      "bot",
    );
    expect(apiClient).toHaveBeenLastCalledWith("/channel-bots/bot/reconnect", {
      method: "POST",
      body: { connection_id: connection },
      signal,
    });
  });
  it("rejects malformed connection IDs before making a request", async () => {
    await expect(
      completeManagedOAuth(
        "x",
        { connection_id: "invalid", label: "Support" },
        signal,
      ),
    ).rejects.toThrow();
    expect(apiClient).not.toHaveBeenCalled();
  });
});

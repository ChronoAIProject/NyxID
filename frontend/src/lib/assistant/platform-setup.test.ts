import { beforeEach, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { nyxAgentTransport } from "./nyxagent-transport";
import { completePlatformSetup } from "./platform-setup";
import {
  getAssistantIdentityRevision,
  transitionAssistantIdentity,
} from "./identity";
import { markAdmitted } from "./send-admission";

vi.mock("@/lib/api-client", () => ({ api: { get: vi.fn() } }));
vi.mock("./nyxagent-transport", () => ({
  nyxAgentTransport: { send: vi.fn() },
}));
const id = "11111111-1111-4111-8111-111111111111";
const source = () => ({
  ownerUserId: "owner",
  identityRevision: getAssistantIdentityRevision(),
  conversationId: `nyxa-${"a".repeat(32)}`,
});
beforeEach(() => {
  vi.resetAllMocks();
  transitionAssistantIdentity("owner");
  vi.mocked(api.get).mockResolvedValue({
    id,
    full_key: "never-send-this-key",
    webhook_secret: "never-send-this-secret",
  });
  vi.mocked(nyxAgentTransport.send).mockResolvedValue(undefined);
});

it.each([
  ["agent_key", "/api-keys/"],
  ["channel_bot", "/channel-bots/"],
] as const)(
  "verifies %s and continues the exact source chat with metadata only",
  async (kind, path) => {
    await completePlatformSetup({ kind, id }, source());
    expect(api.get).toHaveBeenCalledWith(`${path}${id}`, {
      authorityGuard: expect.any(Function),
    });
    expect(nyxAgentTransport.send).toHaveBeenCalledWith(
      source().conversationId,
      expect.stringContaining(id),
      expect.any(Function),
      undefined,
      { adoptExisting: true },
    );
    const text = vi.mocked(nyxAgentTransport.send).mock.calls[0]![1];
    expect(text).toContain("Verify its current state and permissions");
    expect(text).not.toContain("never-send");
  },
);

it("rejects missing or mismatched resource evidence and never claims completion", async () => {
  vi.mocked(api.get).mockResolvedValue({
    id: "22222222-2222-4222-8222-222222222222",
  });
  await expect(
    completePlatformSetup({ kind: "agent_key", id }, source()),
  ).rejects.toThrow("verify");
  expect(nyxAgentTransport.send).not.toHaveBeenCalled();
});

it("fences account changes including switching away and back during readback", async () => {
  const bound = source();
  vi.mocked(api.get).mockImplementation(async () => {
    transitionAssistantIdentity("other");
    transitionAssistantIdentity("owner");
    return { id };
  });
  await expect(
    completePlatformSetup({ kind: "agent_key", id }, bound),
  ).rejects.toThrow("account changed");
  expect(nyxAgentTransport.send).not.toHaveBeenCalled();
});

it("allows a completion retry after rejected admission, retaining the existing resource", async () => {
  vi.mocked(nyxAgentTransport.send).mockRejectedValueOnce(
    new Error("A turn is already active."),
  );
  await expect(
    completePlatformSetup({ kind: "agent_key", id }, source()),
  ).rejects.toThrow("already active");
  await completePlatformSetup({ kind: "agent_key", id }, source());
  expect(nyxAgentTransport.send).toHaveBeenCalledTimes(2);
  expect(
    vi
      .mocked(nyxAgentTransport.send)
      .mock.calls.every((call) => call[0] === source().conversationId),
  ).toBe(true);
});

it("settles an admitted completion without replaying its user message", async () => {
  vi.mocked(nyxAgentTransport.send).mockRejectedValue(
    markAdmitted(new Error("Stream lost")),
  );
  await completePlatformSetup({ kind: "agent_key", id }, source());
  expect(nyxAgentTransport.send).toHaveBeenCalledTimes(1);
});

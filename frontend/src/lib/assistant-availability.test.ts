import { beforeEach, describe, expect, it, vi } from "vitest";
import type { UserCapabilities } from "@/types/api";
import { FEATURE_FLAG } from "./feature-flags";
import {
  fetchAssistantAccessUser,
  hasAssistantAccess,
} from "./assistant-availability";

const { mockGet } = vi.hoisted(() => ({
  mockGet: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet },
}));

function userWithAssistant(enabled: boolean): {
  readonly capabilities: UserCapabilities;
} {
  return {
    capabilities: {
      enabled_features: enabled ? [FEATURE_FLAG.AI_ASSISTANT] : [],
    },
  };
}

describe("hasAssistantAccess", () => {
  it("admits when the server-verified user has the flag", () => {
    expect(hasAssistantAccess(userWithAssistant(true), null)).toBe(true);
  });

  it("denies when the server-verified user lacks the flag, even if the snapshot has it", () => {
    expect(
      hasAssistantAccess(userWithAssistant(false), userWithAssistant(true)),
    ).toBe(false);
  });

  it("falls back to the store snapshot when the verification fetch failed", () => {
    expect(hasAssistantAccess(null, userWithAssistant(true))).toBe(true);
    expect(hasAssistantAccess(null, userWithAssistant(false))).toBe(false);
  });

  it("fails closed when neither source provides a user", () => {
    expect(hasAssistantAccess(null, null)).toBe(false);
  });
});

describe("fetchAssistantAccessUser", () => {
  beforeEach(() => {
    mockGet.mockReset();
  });

  it("returns the server-resolved user from /users/me", async () => {
    const user = userWithAssistant(true);
    mockGet.mockResolvedValueOnce(user);

    await expect(fetchAssistantAccessUser()).resolves.toBe(user);
    expect(mockGet).toHaveBeenCalledWith("/users/me");
  });

  it("resolves to null when the verification fetch rejects", async () => {
    mockGet.mockRejectedValueOnce(new Error("network down"));

    await expect(fetchAssistantAccessUser()).resolves.toBeNull();
  });
});

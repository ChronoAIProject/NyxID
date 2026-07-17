import { beforeEach, describe, expect, it, vi } from "vitest";
import type { User, UserCapabilities } from "@/types/api";
import { FEATURE_FLAG } from "./feature-flags";
import {
  fetchAssistantAccessUser,
  hasAssistantAccess,
  resolveAssistantEntry,
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

function testUser(id: string, assistantEnabled: boolean): User {
  return {
    id,
    email: `${id}@example.com`,
    display_name: "Test User",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: "2026-01-01T00:00:00Z",
    capabilities: {
      enabled_features: assistantEnabled ? [FEATURE_FLAG.AI_ASSISTANT] : [],
    },
  };
}

interface MutableAuth {
  isAuthenticated: boolean;
  isLoading: boolean;
  user: User | null;
}

function entryHarness(initial: MutableAuth) {
  const state: MutableAuth = { ...initial };
  const applyUser = vi.fn((user: User) => {
    state.user = user;
    state.isAuthenticated = true;
  });
  return {
    state,
    applyUser,
    getAuth: () => ({ ...state }),
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

describe("resolveAssistantEntry — enter/preload (server-verified)", () => {
  it("admits and applies the fetched user when the flag is on", async () => {
    const fetched = testUser("u1", true);
    const h = entryHarness({
      isAuthenticated: true,
      isLoading: false,
      user: testUser("u1", true),
    });
    const fetchUser = vi.fn().mockResolvedValue(fetched);

    await expect(
      resolveAssistantEntry({ cause: "enter", ...h, fetchUser }),
    ).resolves.toBe("allow");
    expect(h.applyUser).toHaveBeenCalledWith(fetched);
  });

  it("preload verifies exactly like enter", async () => {
    const fetchUser = vi.fn().mockResolvedValue(testUser("u1", true));
    const h = entryHarness({
      isAuthenticated: true,
      isLoading: false,
      user: null,
    });

    await expect(
      resolveAssistantEntry({ cause: "preload", ...h, fetchUser }),
    ).resolves.toBe("allow");
    expect(fetchUser).toHaveBeenCalledOnce();
  });

  it("redirects to dashboard when the server says the flag is off, even if the snapshot still has it", async () => {
    const h = entryHarness({
      isAuthenticated: true,
      isLoading: false,
      user: testUser("u1", true),
    });
    const fetchUser = vi.fn().mockResolvedValue(testUser("u1", false));

    await expect(
      resolveAssistantEntry({ cause: "enter", ...h, fetchUser }),
    ).resolves.toBe("redirect-dashboard");
  });

  it("falls back to the authenticated snapshot when the fetch fails transiently", async () => {
    const h = entryHarness({
      isAuthenticated: true,
      isLoading: false,
      user: testUser("u1", true),
    });
    const fetchUser = vi.fn().mockResolvedValue(null);

    await expect(
      resolveAssistantEntry({ cause: "enter", ...h, fetchUser }),
    ).resolves.toBe("allow");
    expect(h.applyUser).not.toHaveBeenCalled();
  });

  it("fails closed to dashboard when the fetch fails and the snapshot lacks the flag", async () => {
    const h = entryHarness({
      isAuthenticated: true,
      isLoading: false,
      user: testUser("u1", false),
    });
    const fetchUser = vi.fn().mockResolvedValue(null);

    await expect(
      resolveAssistantEntry({ cause: "enter", ...h, fetchUser }),
    ).resolves.toBe("redirect-dashboard");
  });

  it("redirects to login without fetching when already signed out and settled", async () => {
    const h = entryHarness({
      isAuthenticated: false,
      isLoading: false,
      user: null,
    });
    const fetchUser = vi.fn();

    await expect(
      resolveAssistantEntry({ cause: "enter", ...h, fetchUser }),
    ).resolves.toBe("redirect-login");
    expect(fetchUser).not.toHaveBeenCalled();
  });

  it("never resurrects a session cleared mid-fetch: sign-out wins over the late 200", async () => {
    const h = entryHarness({
      isAuthenticated: true,
      isLoading: false,
      user: testUser("u1", true),
    });
    // A concurrent 401 clears the session while /users/me is in flight.
    const fetchUser = vi.fn().mockImplementation(async () => {
      h.state.isAuthenticated = false;
      h.state.user = null;
      return testUser("u1", true);
    });

    await expect(
      resolveAssistantEntry({ cause: "enter", ...h, fetchUser }),
    ).resolves.toBe("redirect-login");
    expect(h.applyUser).not.toHaveBeenCalled();
  });

  it("discards the fetched user when a different user signed in mid-fetch", async () => {
    const h = entryHarness({
      isAuthenticated: true,
      isLoading: false,
      user: testUser("u1", true),
    });
    // The old session's flagged response arrives after user u2 (no flag)
    // signed in; the stale response must not be applied or trusted.
    const fetchUser = vi.fn().mockImplementation(async () => {
      h.state.user = testUser("u2", false);
      return testUser("u1", true);
    });

    await expect(
      resolveAssistantEntry({ cause: "enter", ...h, fetchUser }),
    ).resolves.toBe("redirect-dashboard");
    expect(h.applyUser).not.toHaveBeenCalled();
  });
});

describe("resolveAssistantEntry — stay (snapshot-only)", () => {
  it("allows without any fetch when the snapshot user has the flag", async () => {
    const h = entryHarness({
      isAuthenticated: true,
      isLoading: false,
      user: testUser("u1", true),
    });
    const fetchUser = vi.fn();

    await expect(
      resolveAssistantEntry({ cause: "stay", ...h, fetchUser }),
    ).resolves.toBe("allow");
    expect(fetchUser).not.toHaveBeenCalled();
  });

  it("evicts to dashboard when the settled snapshot lost the flag mid-session", async () => {
    const h = entryHarness({
      isAuthenticated: true,
      isLoading: false,
      user: testUser("u1", false),
    });

    await expect(
      resolveAssistantEntry({ cause: "stay", ...h, fetchUser: vi.fn() }),
    ).resolves.toBe("redirect-dashboard");
  });

  it("redirects to login when the session died and the store is settled", async () => {
    const h = entryHarness({
      isAuthenticated: false,
      isLoading: false,
      user: null,
    });

    await expect(
      resolveAssistantEntry({ cause: "stay", ...h, fetchUser: vi.fn() }),
    ).resolves.toBe("redirect-login");
  });

  it("never acts on an unsettled store (isLoading)", async () => {
    const h = entryHarness({
      isAuthenticated: false,
      isLoading: true,
      user: null,
    });

    await expect(
      resolveAssistantEntry({ cause: "stay", ...h, fetchUser: vi.fn() }),
    ).resolves.toBe("allow");
  });

  it("allows an authenticated session whose user object has not hydrated yet", async () => {
    const h = entryHarness({
      isAuthenticated: true,
      isLoading: false,
      user: null,
    });

    await expect(
      resolveAssistantEntry({ cause: "stay", ...h, fetchUser: vi.fn() }),
    ).resolves.toBe("allow");
  });
});

import { describe, it, expect, vi, beforeEach } from "vitest";
import { useAuthStore } from "./auth-store";
import { ApiError } from "@/lib/api-client";

vi.mock("@/lib/api-client", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api-client")>(
      "@/lib/api-client",
    );
  return {
    ...actual,
    api: {
      get: vi.fn(),
      post: vi.fn(),
      put: vi.fn(),
      patch: vi.fn(),
      delete: vi.fn(),
    },
  };
});

async function getApiMock() {
  const mod = await import("@/lib/api-client");
  return mod.api as {
    get: ReturnType<typeof vi.fn>;
    post: ReturnType<typeof vi.fn>;
    put: ReturnType<typeof vi.fn>;
    patch: ReturnType<typeof vi.fn>;
    delete: ReturnType<typeof vi.fn>;
  };
}

beforeEach(() => {
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    isLoading: true,
    mfaRequired: false,
    mfaToken: null,
  });
});

describe("useAuthStore initial state", () => {
  it("has correct initial state", () => {
    const state = useAuthStore.getState();
    expect(state.user).toBeNull();
    expect(state.isAuthenticated).toBe(false);
    expect(state.isLoading).toBe(true);
    expect(state.mfaRequired).toBe(false);
    expect(state.mfaToken).toBeNull();
  });
});

describe("setUser", () => {
  it("sets user and marks as authenticated", () => {
    const mockUser = {
      id: "u1",
      email: "test@test.com",
      name: "Test",
      avatar_url: null,
      email_verified: true,
      mfa_enabled: false,
      is_admin: false,
      is_active: true,
      created_at: "2024-01-01T00:00:00Z",
    };

    useAuthStore.getState().setUser(mockUser);
    const state = useAuthStore.getState();
    expect(state.user).toEqual(mockUser);
    expect(state.isAuthenticated).toBe(true);
  });

  it("clears auth when setting user to null", () => {
    useAuthStore.getState().setUser(null);
    const state = useAuthStore.getState();
    expect(state.user).toBeNull();
    expect(state.isAuthenticated).toBe(false);
  });
});

describe("setMfaRequired", () => {
  it("sets MFA state", () => {
    useAuthStore.getState().setMfaRequired(true, "mfa-token-123");
    const state = useAuthStore.getState();
    expect(state.mfaRequired).toBe(true);
    expect(state.mfaToken).toBe("mfa-token-123");
  });
});

describe("clearMfaState", () => {
  it("clears MFA state", () => {
    useAuthStore.getState().setMfaRequired(true, "token");
    useAuthStore.getState().clearMfaState();
    const state = useAuthStore.getState();
    expect(state.mfaRequired).toBe(false);
    expect(state.mfaToken).toBeNull();
  });
});

describe("login", () => {
  it("sets authenticated on success", async () => {
    const apiMock = await getApiMock();
    apiMock.post.mockResolvedValueOnce({
      user_id: "u1",
      access_token: "token",
      expires_in: 3600,
    });

    const result = await useAuthStore.getState().login("a@b.com", "pass");
    expect(result.mfaRequired).toBe(false);
    expect(useAuthStore.getState().isAuthenticated).toBe(true);
  });

  it("sets MFA state on MFA required error", async () => {
    const apiMock = await getApiMock();
    apiMock.post.mockRejectedValueOnce(
      new ApiError(403, {
        error: "mfa_required",
        error_code: 2002,
        message: "MFA required",
        session_token: "session-tok-123",
      } as never),
    );

    const result = await useAuthStore.getState().login("a@b.com", "pass");
    expect(result.mfaRequired).toBe(true);
    expect(useAuthStore.getState().mfaRequired).toBe(true);
    expect(useAuthStore.getState().mfaToken).toBe("session-tok-123");
  });

  it("rethrows non-MFA errors", async () => {
    const apiMock = await getApiMock();
    apiMock.post.mockRejectedValueOnce(
      new ApiError(401, {
        error: "invalid_credentials",
        error_code: 1001,
        message: "Invalid credentials",
      }),
    );

    await expect(
      useAuthStore.getState().login("a@b.com", "bad"),
    ).rejects.toThrow("Invalid credentials");
  });
});

describe("logout", () => {
  it("clears all state after logout", async () => {
    const apiMock = await getApiMock();
    apiMock.post.mockResolvedValueOnce(undefined);

    useAuthStore.setState({ isAuthenticated: true, mfaRequired: true });
    await useAuthStore.getState().logout();

    const state = useAuthStore.getState();
    expect(state.user).toBeNull();
    expect(state.isAuthenticated).toBe(false);
    expect(state.mfaRequired).toBe(false);
  });

  it("clears state even if API call fails", async () => {
    const apiMock = await getApiMock();
    apiMock.post.mockRejectedValueOnce(new Error("Network error"));

    useAuthStore.setState({ isAuthenticated: true });
    try {
      await useAuthStore.getState().logout();
    } catch {
      // Expected - logout rethrows after finally
    }

    expect(useAuthStore.getState().isAuthenticated).toBe(false);
  });
});

describe("checkAuth", () => {
  it("sets user on success", async () => {
    const apiMock = await getApiMock();
    const mockUser = {
      id: "u1",
      email: "test@test.com",
      name: "Test",
      avatar_url: null,
      email_verified: true,
      mfa_enabled: false,
      is_admin: false,
      is_active: true,
      created_at: "2024-01-01T00:00:00Z",
    };
    apiMock.get.mockResolvedValueOnce(mockUser);

    await useAuthStore.getState().checkAuth();
    const state = useAuthStore.getState();
    expect(state.user).toEqual(mockUser);
    expect(state.isAuthenticated).toBe(true);
    expect(state.isLoading).toBe(false);
  });

  it("clears user on 401", async () => {
    const apiMock = await getApiMock();
    apiMock.get.mockRejectedValueOnce(
      new ApiError(401, {
        error: "unauthorized",
        error_code: 1001,
        message: "Not authenticated",
      }),
    );

    await useAuthStore.getState().checkAuth();
    const state = useAuthStore.getState();
    expect(state.user).toBeNull();
    expect(state.isAuthenticated).toBe(false);
    expect(state.isLoading).toBe(false);
  });

  it("only clears loading on non-401 errors", async () => {
    const apiMock = await getApiMock();
    apiMock.get.mockRejectedValueOnce(
      new ApiError(500, {
        error: "server_error",
        error_code: 5000,
        message: "Server error",
      }),
    );

    await useAuthStore.getState().checkAuth();
    const state = useAuthStore.getState();
    expect(state.isLoading).toBe(false);
    // user stays null since initial state
    expect(state.isAuthenticated).toBe(false);
  });
});

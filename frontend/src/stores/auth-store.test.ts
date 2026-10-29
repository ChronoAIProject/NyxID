import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { useAuthStore } from "./auth-store";
import { useAssistantContextStore } from "./assistant-context-store";
import { useAssistantDraftStore } from "./assistant-draft-store";
import { useAssistantWireLogStore } from "./assistant-wire-log-store";
import { useServiceCardView } from "./service-card-view-store";
import { useConsentStore } from "./consent-store";
import { ApiError } from "@/lib/api-client";
import {
  identify as telemetryIdentify,
  reset as telemetryReset,
} from "@/lib/telemetry";
import {
  getAssistantIdentityUserId,
  transitionAssistantIdentity,
} from "@/lib/assistant/identity";

vi.mock("@/lib/telemetry", async () => ({
  ...(await vi.importActual<typeof import("@/lib/telemetry")>(
    "@/lib/telemetry",
  )),
  identify: vi.fn(),
  reset: vi.fn(),
}));

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
  vi.clearAllMocks();
  localStorage.clear();
  transitionAssistantIdentity(null);
  useServiceCardView.setState({
    accountId: undefined,
    filters: undefined,
    expanded: [],
    savedViewId: undefined,
  });
  useAssistantContextStore.setState({
    ownerUserId: null,
    lastScreen: null,
  });
  useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
  useAssistantWireLogStore.setState({
    featureEnabled: false,
    captureEnabled: false,
    showResponses: true,
    entries: [],
    totalBytes: 0,
    captureBytes: 0,
  });
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    isLoading: true,
    mfaRequired: false,
    mfaToken: null,
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const previousAccount = {
  id: "previous-person",
  email: "previous@example.com",
  display_name: null,
  avatar_url: null,
  email_verified: true,
  mfa_enabled: true,
  is_admin: false,
  is_active: true,
  created_at: "2026-01-01T00:00:00Z",
};

describe("useAuthStore initial state", () => {
  it.each([401, 503])(
    "retains pending password MFA when profile verification fails (%i)",
    async (status) => {
      const api = await getApiMock();
      useAuthStore.getState().setMfaRequired(true, "pending-password-mfa");
      api.get.mockRejectedValueOnce(
        new ApiError(status, {
          error: "failed",
          error_code: 1000,
          message: "Failed",
        }),
      );
      await useAuthStore.getState().checkAuth();
      expect(useAuthStore.getState().mfaRequired).toBe(true);
      expect(useAuthStore.getState().mfaToken).toBe("pending-password-mfa");
    },
  );

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
      display_name: "Test",
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
    });

    const result = await useAuthStore.getState().login("a@b.com", "pass");
    expect(result.mfaRequired).toBe(false);
    expect(useAuthStore.getState().isAuthenticated).toBe(true);
    expect(apiMock.post).toHaveBeenCalledWith("/auth/login", {
      email: "a@b.com",
      password: "pass",
      client: "web",
    });
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
    useAssistantContextStore.getState().recordScreen("u1", "/keys");
    useAssistantDraftStore
      .getState()
      .saveDraft("u1", "conv:conversation-private", "Private draft");
    useAssistantWireLogStore.getState().setFeatureEnabled(true);
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    useAssistantWireLogStore.getState().recordExchange({
      kind: "sse",
      status: 200,
      conversationId: "conversation-private",
      wireLogId: null,
      label: "POST /assistant/chat",
      envelopes: [
        {
          degraded: false,
          method: "POST",
          path: "api/chat",
          commandType: "text",
          body: { prompt: "Private prompt" },
          headers: {},
          identity: {
            mode: "jwt",
            forward_access_token: false,
            inject_delegation_token: true,
            bridge_minted: false,
          },
          truncated: false,
          response: null,
          upstreamOutcome: "no_response",
        },
      ],
    });
    expect(useAssistantWireLogStore.getState().entries).toHaveLength(1);
    await useAuthStore.getState().logout();

    const state = useAuthStore.getState();
    expect(state.user).toBeNull();
    expect(state.isAuthenticated).toBe(false);
    expect(state.mfaRequired).toBe(false);
    expect(useAssistantContextStore.getState()).toMatchObject({
      ownerUserId: null,
      lastScreen: null,
    });
    expect(useAssistantDraftStore.getState().drafts).toEqual({});
    expect(useAssistantWireLogStore.getState().entries).toEqual([]);
    expect(useAssistantWireLogStore.getState().totalBytes).toBe(0);
    expect(useAssistantWireLogStore.getState().captureBytes).toBe(0);
    expect(useAssistantWireLogStore.getState().captureEnabled).toBe(false);
    expect(localStorage.getItem("nyxid.assistant_context")).toBeNull();
    expect(localStorage.getItem("nyxid.assistant_drafts")).toBeNull();
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
      display_name: "Test",
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
    useAuthStore.getState().setUser(previousAccount);
    vi.mocked(telemetryReset).mockClear();
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
    expect(state.user).toEqual(previousAccount);
    expect(state.isAuthenticated).toBe(true);
    expect(telemetryReset).not.toHaveBeenCalled();
  });
});

describe("scoped QR session restoration", () => {
  it.each([
    [false, 200],
    [false, 401],
    [false, 503],
    [true, 200],
    [true, 401],
    [true, 503],
  ] as const)(
    "clears unconfirmed identity before profile, preserving global loading %s through status %s",
    async (isLoading, status) => {
      useAuthStore.getState().setUser(previousAccount);
      useAuthStore.getState().setMfaRequired(true, "pending-password-mfa");
      useAuthStore.setState({ isLoading });
      useServiceCardView.setState({
        accountId: previousAccount.id,
        expanded: ["private-service"],
        savedViewId: "private-view",
      });
      useAssistantContextStore
        .getState()
        .recordScreen(previousAccount.id, "/keys");
      useAssistantDraftStore
        .getState()
        .saveDraft(previousAccount.id, "conv:private", "Private draft");
      useAssistantWireLogStore.getState().setFeatureEnabled(true);
      useAssistantWireLogStore.getState().setCaptureEnabled(true);
      useAssistantWireLogStore.getState().recordExchange({
        kind: "sse",
        status: 200,
        conversationId: "private",
        wireLogId: "private-wire-log",
        label: "POST /assistant/chat",
      });
      expect(useAssistantWireLogStore.getState().entries).toHaveLength(1);
      vi.mocked(telemetryIdentify).mockClear();
      vi.mocked(telemetryReset).mockClear();
      let finishProfile!: (response: Response) => void;
      const response = new Promise<Response>((resolve) => {
        finishProfile = resolve;
      });
      vi.stubGlobal("fetch", vi.fn().mockReturnValue(response));

      const restoration = useAuthStore
        .getState()
        .checkAuth({ isCurrent: () => true });
      expect(useAuthStore.getState()).toMatchObject({
        user: null,
        isAuthenticated: false,
        isLoading,
        mfaRequired: true,
        mfaToken: "pending-password-mfa",
      });
      expect(telemetryReset).toHaveBeenCalledOnce();
      expect(telemetryIdentify).not.toHaveBeenCalled();
      expect(getAssistantIdentityUserId()).toBeNull();
      expect(useConsentStore.getState().accountId).toBeNull();
      expect(useConsentStore.getState().enabled).toBe(false);
      expect(useServiceCardView.getState()).toMatchObject({
        accountId: undefined,
        filters: undefined,
        expanded: [],
        savedViewId: undefined,
      });
      expect(useAssistantContextStore.getState()).toMatchObject({
        ownerUserId: null,
        lastScreen: null,
      });
      expect(useAssistantDraftStore.getState()).toMatchObject({
        ownerUserId: null,
        drafts: {},
      });
      expect(useAssistantWireLogStore.getState()).toMatchObject({
        entries: [],
        captureEnabled: false,
        totalBytes: 0,
        captureBytes: 0,
      });
      expect(localStorage.getItem("nyxid.assistant_context")).toBeNull();
      expect(localStorage.getItem("nyxid.assistant_drafts")).toBeNull();

      const nextAccount = {
        ...previousAccount,
        id: "qr-person",
        email: "qr@example.com",
      };
      finishProfile(
        new Response(
          JSON.stringify(
            status === 200
              ? nextAccount
              : {
                  error: "profile_unavailable",
                  error_code: 1001,
                  message: "Profile unavailable",
                },
          ),
          { status },
        ),
      );
      expect(await restoration).toBe(status === 200);
      expect(useAuthStore.getState()).toMatchObject({
        user: status === 200 ? nextAccount : null,
        isAuthenticated: status === 200,
        isLoading,
        mfaRequired: status !== 200,
        mfaToken: status === 200 ? null : "pending-password-mfa",
      });
      expect(telemetryReset).toHaveBeenCalledTimes(status === 503 ? 1 : 2);
      expect(useConsentStore.getState().accountId).toBe(
        status === 200 ? nextAccount.id : null,
      );
      if (status === 200) {
        expect(telemetryIdentify).toHaveBeenCalledExactlyOnceWith(
          nextAccount.id,
        );
        expect(getAssistantIdentityUserId()).toBe(nextAccount.id);
      } else {
        expect(telemetryIdentify).not.toHaveBeenCalled();
        expect(getAssistantIdentityUserId()).toBeNull();
      }
    },
  );

  it("does not begin a cancelled handoff or clear its newer identity", async () => {
    useAuthStore.getState().setUser(previousAccount);
    useAuthStore.getState().setMfaRequired(true, "newer-password-mfa");
    const state = useAuthStore.getState();
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    vi.mocked(telemetryIdentify).mockClear();
    vi.mocked(telemetryReset).mockClear();
    const consent = useConsentStore.getState();
    expect(await state.checkAuth({ isCurrent: () => false })).toBe(false);
    expect(useAuthStore.getState()).toBe(state);
    expect(useConsentStore.getState()).toBe(consent);
    expect(getAssistantIdentityUserId()).toBe(previousAccount.id);
    expect(telemetryReset).not.toHaveBeenCalled();
    expect(telemetryIdentify).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe("ephemeral profile restoration", () => {
  it.each([200, 401, 503])(
    "preserves its existing identity/loading/telemetry semantics through status %s",
    async (status) => {
      useAuthStore.getState().setUser(previousAccount);
      useAuthStore.getState().setMfaRequired(true, "pending-password-mfa");
      useAuthStore.setState({ isLoading: false });
      useAssistantContextStore
        .getState()
        .recordScreen(previousAccount.id, "/keys");
      const accountContext = useAssistantContextStore.getState();
      vi.mocked(telemetryIdentify).mockClear();
      vi.mocked(telemetryReset).mockClear();
      let finishProfile!: (response: Response) => void;
      const response = new Promise<Response>((resolve) => {
        finishProfile = resolve;
      });
      vi.stubGlobal("fetch", vi.fn().mockReturnValue(response));
      const restoration = useAuthStore
        .getState()
        .checkAuth({ ephemeral: true });
      expect(useAuthStore.getState()).toMatchObject({
        user: previousAccount,
        isAuthenticated: true,
        isLoading: true,
      });
      finishProfile(
        new Response(
          JSON.stringify(
            status === 200
              ? previousAccount
              : {
                  error: "profile_unavailable",
                  error_code: 1001,
                  message: "Profile unavailable",
                },
          ),
          { status },
        ),
      );
      expect(await restoration).toBe(status === 200);
      expect(useAuthStore.getState()).toMatchObject({
        user: status === 401 ? null : previousAccount,
        isAuthenticated: status !== 401,
        isLoading: false,
        mfaRequired: status !== 200,
        mfaToken: status === 200 ? null : "pending-password-mfa",
      });
      expect(useAssistantContextStore.getState()).toBe(accountContext);
      expect(telemetryReset).not.toHaveBeenCalled();
      expect(telemetryIdentify).not.toHaveBeenCalled();
    },
  );
});

describe("required session restoration", () => {
  it("rethrows a failed fresh profile for MFA completion", async () => {
    const apiMock = await getApiMock();
    const error = new ApiError(503, {
      error: "unavailable",
      error_code: 1001,
      message: "Unavailable",
    });
    apiMock.get.mockRejectedValueOnce(error);
    await expect(
      useAuthStore.getState().checkAuth({ requireSession: true }),
    ).rejects.toBe(error);
    expect(useAuthStore.getState().isLoading).toBe(false);
  });
});

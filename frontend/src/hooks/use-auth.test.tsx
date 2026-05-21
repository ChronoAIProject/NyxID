import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useLogin,
  useLogout,
  useMfaDisable,
  useMfaSetup,
  useMfaVerify,
  useRegister,
  useUser,
} from "./use-auth";

const { mockGet, mockPost, storeFns } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  storeFns: {
    login: vi.fn(),
    logout: vi.fn(),
    setUser: vi.fn(),
    clearMfaState: vi.fn(),
  },
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
}));

// `use-auth` reads store actions via selectors: useAuthStore((s) => s.login).
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (s: typeof storeFns) => unknown) =>
    selector(storeFns),
}));

function setup() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
  const clearSpy = vi.spyOn(queryClient, "clear");
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return { wrapper, queryClient, invalidateSpy, clearSpy };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useUser", () => {
  it("fetches /users/me and pushes the result into the auth store", async () => {
    const user = { id: "user-1", email: "a@b.com" };
    mockGet.mockResolvedValue(user);

    const { wrapper } = setup();
    const { result } = renderHook(() => useUser(), { wrapper });

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });
    expect(mockGet).toHaveBeenCalledWith("/users/me");
    expect(storeFns.setUser).toHaveBeenCalledWith(user);
    expect(result.current.data).toEqual(user);
  });
});

describe("useRegister", () => {
  it("posts the full credential set to /auth/register", async () => {
    mockPost.mockResolvedValue({ user_id: "user-1" });
    const { wrapper } = setup();
    const { result } = renderHook(() => useRegister(), { wrapper });

    const credentials = {
      email: "new@example.com",
      password: "hunter2hunter2",
      display_name: "New User",
      invite_code: "INVITE-123",
    };
    await result.current.mutateAsync(credentials);

    expect(mockPost).toHaveBeenCalledWith("/auth/register", credentials);
  });
});

describe("useLogin", () => {
  it("invalidates the user query when MFA is not required", async () => {
    storeFns.login.mockResolvedValue({ mfaRequired: false });
    const { wrapper, invalidateSpy } = setup();
    const { result } = renderHook(() => useLogin(), { wrapper });

    await result.current.mutateAsync({
      email: "a@b.com",
      password: "pw",
    });

    expect(storeFns.login).toHaveBeenCalledWith("a@b.com", "pw");
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["user"] });
  });

  it("does NOT invalidate the user query when MFA is required", async () => {
    storeFns.login.mockResolvedValue({ mfaRequired: true });
    const { wrapper, invalidateSpy } = setup();
    const { result } = renderHook(() => useLogin(), { wrapper });

    const res = await result.current.mutateAsync({
      email: "a@b.com",
      password: "pw",
    });

    expect(res.mfaRequired).toBe(true);
    expect(invalidateSpy).not.toHaveBeenCalled();
  });
});

describe("useLogout", () => {
  it("delegates to the store and clears the query cache on success", async () => {
    storeFns.logout.mockResolvedValue(undefined);
    const { wrapper, clearSpy } = setup();
    const { result } = renderHook(() => useLogout(), { wrapper });

    await result.current.mutateAsync();

    expect(storeFns.logout).toHaveBeenCalledTimes(1);
    expect(clearSpy).toHaveBeenCalledTimes(1);
  });
});

describe("useMfaSetup", () => {
  it("posts to /auth/mfa/setup", async () => {
    // Mirror the real MfaSetupResponse shape so a field rename is caught.
    const setupResponse = {
      factor_id: "factor-1",
      secret: "S",
      qr_code_url: "otpauth://totp/NyxID",
    };
    mockPost.mockResolvedValue(setupResponse);
    const { wrapper } = setup();
    const { result } = renderHook(() => useMfaSetup(), { wrapper });

    const data = await result.current.mutateAsync();

    expect(mockPost).toHaveBeenCalledWith("/auth/mfa/setup");
    expect(data).toEqual(setupResponse);
  });
});

describe("useMfaVerify", () => {
  it("injects client:web, clears MFA state and invalidates the user query", async () => {
    mockPost.mockResolvedValue(undefined);
    const { wrapper, invalidateSpy } = setup();
    const { result } = renderHook(() => useMfaVerify(), { wrapper });

    await result.current.mutateAsync({ code: "123456", mfa_token: "mfa-tok" });

    expect(mockPost).toHaveBeenCalledWith("/auth/mfa/verify", {
      code: "123456",
      mfa_token: "mfa-tok",
      client: "web",
    });
    expect(storeFns.clearMfaState).toHaveBeenCalledTimes(1);
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["user"] });
  });
});

describe("useMfaDisable", () => {
  it("posts the password to /auth/mfa/disable and invalidates the user query", async () => {
    mockPost.mockResolvedValue(undefined);
    const { wrapper, invalidateSpy } = setup();
    const { result } = renderHook(() => useMfaDisable(), { wrapper });

    await result.current.mutateAsync("my-password");

    expect(mockPost).toHaveBeenCalledWith("/auth/mfa/disable", {
      password: "my-password",
    });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["user"] });
  });
});

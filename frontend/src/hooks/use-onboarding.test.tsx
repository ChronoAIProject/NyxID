import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import {
  useCompleteOnboarding,
  useShouldShowOnboarding,
} from "./use-onboarding";

const { mockPost } = vi.hoisted(() => ({ mockPost: vi.fn() }));

vi.mock("@/lib/api-client", () => ({
  api: { post: mockPost },
}));

function wrapperFactory() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

function makeUser(
  profileConfig: User["profile_config"] | undefined,
): User {
  return {
    id: "u1",
    email: "u1@x.com",
    profile_config: profileConfig,
  } as User;
}

beforeEach(() => {
  vi.clearAllMocks();
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    isLoading: true,
    mfaRequired: false,
    mfaToken: null,
  });
});

describe("useShouldShowOnboarding gate", () => {
  it("returns `loading` while auth is still loading", () => {
    useAuthStore.setState({ isLoading: true });
    const { result } = renderHook(() => useShouldShowOnboarding(), {
      wrapper: wrapperFactory(),
    });
    expect(result.current.status).toBe("loading");
  });

  it("returns `hidden` when settled but unauthenticated", () => {
    useAuthStore.setState({
      isLoading: false,
      isAuthenticated: false,
      user: null,
    });
    const { result } = renderHook(() => useShouldShowOnboarding(), {
      wrapper: wrapperFactory(),
    });
    expect(result.current.status).toBe("hidden");
  });

  it("fails open to `hidden` when profile_config is absent (older backends)", () => {
    useAuthStore.setState({
      isLoading: false,
      isAuthenticated: true,
      user: makeUser(undefined),
    });
    const { result } = renderHook(() => useShouldShowOnboarding(), {
      wrapper: wrapperFactory(),
    });
    expect(result.current.status).toBe("hidden");
  });

  it("returns `show` when the AI-services flow has no completion timestamp", () => {
    useAuthStore.setState({
      isLoading: false,
      isAuthenticated: true,
      user: makeUser({ onboarding: { ai_services_completed_at: null } }),
    });
    const { result } = renderHook(() => useShouldShowOnboarding(), {
      wrapper: wrapperFactory(),
    });
    expect(result.current.status).toBe("show");
  });

  it("returns `hidden` once the AI-services flow has a completion timestamp", () => {
    useAuthStore.setState({
      isLoading: false,
      isAuthenticated: true,
      user: makeUser({
        onboarding: { ai_services_completed_at: "2026-01-01T00:00:00Z" },
      }),
    });
    const { result } = renderHook(() => useShouldShowOnboarding(), {
      wrapper: wrapperFactory(),
    });
    expect(result.current.status).toBe("hidden");
  });
});

describe("useCompleteOnboarding", () => {
  it("POSTs the flow key to the complete endpoint", async () => {
    mockPost.mockResolvedValue({ ai_services_completed_at: "2026-01-01T00:00:00Z" });
    const { result } = renderHook(() => useCompleteOnboarding(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({ key: "ai-services-wizard" });
    expect(mockPost).toHaveBeenCalledWith("/users/me/onboarding/complete", {
      key: "ai-services-wizard",
    });
  });
});

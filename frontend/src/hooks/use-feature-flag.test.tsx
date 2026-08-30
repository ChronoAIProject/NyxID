import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PropsWithChildren } from "react";
import { useFeature } from "./use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";

const { mockGet } = vi.hoisted(() => ({ mockGet: vi.fn() }));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet },
}));

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

function orgResponse(enabledFeatures: string[]) {
  return {
    id: "org-1",
    slug: "acme",
    display_name: "Acme",
    avatar_url: null,
    contact_email: null,
    created_at: "2026-01-01T00:00:00Z",
    remote_credential_integrity_verification_opt_out: false,
    your_role: "member",
    member_count: 1,
    enabled_features: enabledFeatures,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useFeature", () => {
  it("is true when the org's enabled_features includes the flag", async () => {
    mockGet.mockResolvedValue(orgResponse([FEATURE_FLAG.AI_ASSISTANT]));
    const { result } = renderHook(
      () => useFeature(FEATURE_FLAG.AI_ASSISTANT, "org-1"),
      { wrapper: createWrapper() },
    );
    await waitFor(() => expect(result.current).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/orgs/org-1");
  });

  it("is false when the flag is absent from enabled_features", async () => {
    mockGet.mockResolvedValue(orgResponse([]));
    const { result } = renderHook(
      () => useFeature(FEATURE_FLAG.AI_ASSISTANT, "org-1"),
      { wrapper: createWrapper() },
    );
    // Let the query settle, then assert still-false (fail-closed).
    await waitFor(() => expect(mockGet).toHaveBeenCalled());
    expect(result.current).toBe(false);
  });

  it("fails closed to false while loading / without an orgId", () => {
    mockGet.mockResolvedValue(orgResponse([FEATURE_FLAG.AI_ASSISTANT]));
    const { result } = renderHook(
      () => useFeature(FEATURE_FLAG.AI_ASSISTANT, ""),
      { wrapper: createWrapper() },
    );
    expect(result.current).toBe(false);
    expect(mockGet).not.toHaveBeenCalled();
  });
});

describe("useFeature (personal / non-org context)", () => {
  afterEach(() => {
    useAuthStore.setState({ user: null });
  });

  it("reads capabilities.enabled_features from /users/me when no orgId", () => {
    useAuthStore.setState({
      user: {
        capabilities: { enabled_features: [FEATURE_FLAG.AI_ASSISTANT] },
      } as never,
    });
    const { result } = renderHook(() => useFeature(FEATURE_FLAG.AI_ASSISTANT), {
      wrapper: createWrapper(),
    });
    expect(result.current).toBe(true);
    expect(mockGet).not.toHaveBeenCalled(); // no org fetch in personal context
  });

  it("fails closed when the personal set lacks the flag", () => {
    useAuthStore.setState({
      user: { capabilities: { enabled_features: [] } } as never,
    });
    const { result } = renderHook(() => useFeature(FEATURE_FLAG.AI_ASSISTANT), {
      wrapper: createWrapper(),
    });
    expect(result.current).toBe(false);
  });
});

describe("feature flag catalog", () => {
  // The catalog keys are a wire contract with
  // `backend/src/services/feature_flag_service.rs::FEATURE_FLAGS`; a key that
  // drifts on one side silently disables the surface it gates instead of
  // failing. Pinning the literals makes a rename a deliberate two-sided edit.
  it("pins the backend registry key literals", () => {
    expect(FEATURE_FLAG).toEqual({
      AI_ASSISTANT: "experimental:ai-assistant",
      BILLING: "experimental:billing",
      AEVATAR_CHAT_WIRE_LOG: "experimental:aevatar-chat-wire-log",
      DIRECT_CHAT_ENGINE: "experimental:direct-chat-engine",
      PLATFORM_SERVICES: "experimental:platform-services",
    });
  });
});

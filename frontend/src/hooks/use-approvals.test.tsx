import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useApprovalGrants,
  useApprovalRequests,
  useDecideApproval,
  useDeleteServiceApprovalConfig,
  useNotificationSettings,
  usePushDevices,
  useRemoveDevice,
  useRevokeGrant,
  useServiceApprovalConfigs,
  useSetServiceApprovalConfig,
  useTelegramDisconnect,
  useTelegramLink,
  useUpdateNotificationSettings,
} from "./use-approvals";

const { mockGet, mockPost, mockPut, mockDelete } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockPut: vi.fn(),
  mockDelete: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost, put: mockPut, delete: mockDelete },
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

beforeEach(() => {
  vi.clearAllMocks();
  mockGet.mockResolvedValue({});
});

describe("notification + device hooks", () => {
  it("useNotificationSettings GETs /notifications/settings", async () => {
    const { result } = renderHook(() => useNotificationSettings(), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/notifications/settings");
  });

  it("useUpdateNotificationSettings PUTs the settings body", async () => {
    mockPut.mockResolvedValue({});
    const { result } = renderHook(() => useUpdateNotificationSettings(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({ telegram_enabled: true });
    expect(mockPut).toHaveBeenCalledWith("/notifications/settings", {
      telegram_enabled: true,
    });
  });

  it("useTelegramLink POSTs /notifications/telegram/link", async () => {
    mockPost.mockResolvedValue({ link_url: "https://t.me/x" });
    const { result } = renderHook(() => useTelegramLink(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync();
    expect(mockPost).toHaveBeenCalledWith("/notifications/telegram/link");
  });

  it("useTelegramDisconnect DELETEs /notifications/telegram", async () => {
    mockDelete.mockResolvedValue({});
    const { result } = renderHook(() => useTelegramDisconnect(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync();
    expect(mockDelete).toHaveBeenCalledWith("/notifications/telegram");
  });

  it("usePushDevices GETs /notifications/devices", async () => {
    const { result } = renderHook(() => usePushDevices(), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/notifications/devices");
  });

  it("useRemoveDevice DELETEs the specific device", async () => {
    mockDelete.mockResolvedValue({});
    const { result } = renderHook(() => useRemoveDevice(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync("device-9");
    expect(mockDelete).toHaveBeenCalledWith("/notifications/devices/device-9");
  });
});

describe("useApprovalRequests query string", () => {
  it("encodes pagination and omits status when not provided", async () => {
    const { result } = renderHook(() => useApprovalRequests(2, 50), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/approvals/requests?page=2&per_page=50",
    );
  });

  it("appends status when provided", async () => {
    const { result } = renderHook(
      () => useApprovalRequests(1, 20, "pending"),
      { wrapper: wrapperFactory() },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/approvals/requests?page=1&per_page=20&status=pending",
    );
  });
});

describe("useDecideApproval", () => {
  it("posts approved=true (approve) to the decide endpoint", async () => {
    mockPost.mockResolvedValue({});
    const { result } = renderHook(() => useDecideApproval(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({ requestId: "req-1", approved: true });
    expect(mockPost).toHaveBeenCalledWith(
      "/approvals/requests/req-1/decide",
      { approved: true },
    );
  });

  it("posts approved=false (deny) to the decide endpoint", async () => {
    mockPost.mockResolvedValue({});
    const { result } = renderHook(() => useDecideApproval(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({ requestId: "req-2", approved: false });
    expect(mockPost).toHaveBeenCalledWith(
      "/approvals/requests/req-2/decide",
      { approved: false },
    );
  });
});

describe("useApprovalGrants query string", () => {
  it("omits org_id for the personal scope", async () => {
    const { result } = renderHook(() => useApprovalGrants(1, 20), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/approvals/grants?page=1&per_page=20");
  });

  it("appends org_id when listing an org's grants", async () => {
    const { result } = renderHook(() => useApprovalGrants(1, 20, "org-1"), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/approvals/grants?page=1&per_page=20&org_id=org-1",
    );
  });
});

describe("useRevokeGrant path selection", () => {
  it("uses the bare path for personal grants", async () => {
    mockDelete.mockResolvedValue({});
    const { result } = renderHook(() => useRevokeGrant(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({ grantId: "g1" });
    expect(mockDelete).toHaveBeenCalledWith("/approvals/grants/g1");
  });

  it("encodes the org_id query param for org grants", async () => {
    mockDelete.mockResolvedValue({});
    const { result } = renderHook(() => useRevokeGrant(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({ grantId: "g1", orgId: "org/a" });
    expect(mockDelete).toHaveBeenCalledWith(
      "/approvals/grants/g1?org_id=org%2Fa",
    );
  });
});

describe("per-service approval configs", () => {
  it("useServiceApprovalConfigs lists personal configs by default", async () => {
    const { result } = renderHook(() => useServiceApprovalConfigs(), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/approvals/service-configs");
  });

  it("useServiceApprovalConfigs scopes to an org when given one", async () => {
    const { result } = renderHook(() => useServiceApprovalConfigs("org/a"), {
      wrapper: wrapperFactory(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith(
      "/approvals/service-configs?org_id=org%2Fa",
    );
  });

  it("useSetServiceApprovalConfig sends only the fields that were provided", async () => {
    mockPut.mockResolvedValue({});
    const { result } = renderHook(() => useSetServiceApprovalConfig(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({
      serviceId: "svc-1",
      approvalRequired: true,
      approvalMode: "grant",
    });
    expect(mockPut).toHaveBeenCalledWith("/approvals/service-configs/svc-1", {
      approval_required: true,
      approval_mode: "grant",
    });
  });

  it("useSetServiceApprovalConfig forwards approval_required:false (not just true)", async () => {
    mockPut.mockResolvedValue({});
    const { result } = renderHook(() => useSetServiceApprovalConfig(), {
      wrapper: wrapperFactory(),
    });
    // The `!== undefined` guard must keep an explicit `false`; a truthy
    // check would silently drop it and make approval impossible to turn off.
    await result.current.mutateAsync({
      serviceId: "svc-1",
      approvalRequired: false,
    });
    expect(mockPut).toHaveBeenCalledWith("/approvals/service-configs/svc-1", {
      approval_required: false,
    });
  });

  it("useSetServiceApprovalConfig forwards granular rules and default effect", async () => {
    mockPut.mockResolvedValue({});
    const { result } = renderHook(() => useSetServiceApprovalConfig(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({
      serviceId: "svc-1",
      approvalMode: "per_request",
      rules: [
        {
          methods: [],
          resource_pattern: "*",
          verbs: ["write"],
          effect: "require_approval",
          mode: "per_request",
        },
      ],
      defaultEffect: "auto_allow",
    });
    expect(mockPut).toHaveBeenCalledWith("/approvals/service-configs/svc-1", {
      approval_mode: "per_request",
      rules: [
        {
          methods: [],
          resource_pattern: "*",
          verbs: ["write"],
          effect: "require_approval",
          mode: "per_request",
        },
      ],
      default_effect: "auto_allow",
    });
  });

  it("useSetServiceApprovalConfig omits unset fields and routes through the org path", async () => {
    mockPut.mockResolvedValue({});
    const { result } = renderHook(() => useSetServiceApprovalConfig(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({
      serviceId: "svc-1",
      approvalMode: "per_request",
      orgId: "org-1",
    });
    expect(mockPut).toHaveBeenCalledWith(
      "/approvals/service-configs/svc-1?org_id=org-1",
      { approval_mode: "per_request" },
    );
  });

  it("useDeleteServiceApprovalConfig deletes the personal config", async () => {
    mockDelete.mockResolvedValue({});
    const { result } = renderHook(() => useDeleteServiceApprovalConfig(), {
      wrapper: wrapperFactory(),
    });
    await result.current.mutateAsync({ serviceId: "svc-1" });
    expect(mockDelete).toHaveBeenCalledWith("/approvals/service-configs/svc-1");
  });
});

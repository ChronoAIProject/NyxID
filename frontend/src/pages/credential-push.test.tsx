import type { ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DownstreamService } from "@/types/api";
import type { NodeInfo } from "@/types/nodes";
import { NodeDetailPage } from "./node-detail";
import { ServiceDetailPage } from "./service-detail";

const {
  hooks,
  mockNavigate,
  mockPushCredential,
  mockToastError,
  mockToastSuccess,
  routerState,
} = vi.hoisted(() => {
  const hooks = {
    node: {
      data: undefined as NodeInfo | undefined,
      isLoading: false,
      error: null as unknown,
      refetch: vi.fn(),
    },
    admins: { data: [], isLoading: false },
    pendingCredentials: { data: [], isLoading: false },
    keys: { data: [] },
    service: {
      data: undefined as DownstreamService | undefined,
      isLoading: false,
      error: null as unknown,
      refetch: vi.fn(),
    },
  };

  return {
    hooks,
    mockNavigate: vi.fn(),
    mockPushCredential: vi.fn(),
    mockToastError: vi.fn(),
    mockToastSuccess: vi.fn(),
    routerState: {
      params: { nodeId: "node-1", serviceId: "svc-1" },
    },
  };
});

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    ...props
  }: {
    readonly children: ReactNode;
    readonly to: string;
  }) => (
    <a href={to} {...props}>
      {children}
    </a>
  ),
  useNavigate: () => mockNavigate,
  useParams: () => routerState.params,
}));

vi.mock("@/hooks/use-nodes", () => ({
  useNode: () => hooks.node,
  useNodeAdmins: () => hooks.admins,
  useNodePendingCredentials: () => hooks.pendingCredentials,
  useDeleteNode: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useRotateNodeToken: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useTransferNode: () => ({ mutateAsync: vi.fn(), isPending: false }),
  usePushNodeCredential: () => ({
    mutateAsync: mockPushCredential,
    isPending: false,
  }),
  useCancelNodePendingCredential: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => hooks.keys,
}));

vi.mock("@/hooks/use-services", () => ({
  useService: () => hooks.service,
  useDeleteService: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useTestSshConnection: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));

vi.mock("@/hooks/use-providers", () => ({
  useMyProviderTokens: () => ({ data: [] }),
}));

vi.mock("@/hooks/use-developer-apps", () => ({
  useDeveloperApps: () => ({ data: { clients: [] } }),
}));

vi.mock("@/hooks/use-anonymous-endpoints", () => ({
  useAnonymousEndpoints: () => ({ data: [], isLoading: false }),
  useCreateAnonymousEndpoint: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useUpdateAnonymousEndpoint: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useDeleteAnonymousEndpoint: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: { id: string } }) => unknown) =>
    selector({ user: { id: "user-1" } }),
}));

vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: () => {},
}));

vi.mock("@/components/dashboard/routing-section", () => ({
  RoutingSection: () => <div data-testid="routing-section">Routing</div>,
}));

vi.mock("@/components/shared/default-headers-editor", () => ({
  DefaultHeadersEditor: () => <div data-testid="default-headers-editor" />,
}));

vi.mock("@/components/dashboard/endpoint-list", () => ({
  EndpointList: () => <div data-testid="endpoint-list" />,
}));

vi.mock("@/components/dashboard/mcp-connection-info", () => ({
  McpConnectionInfo: () => <div data-testid="mcp-connection" />,
}));

vi.mock("@/components/dashboard/oidc-credentials-section", () => ({
  OidcCredentialsSection: () => <div data-testid="oidc-credentials" />,
}));

vi.mock("@/components/dashboard/ssh-service-instructions", () => ({
  SshServiceInstructions: () => <div data-testid="ssh-instructions" />,
}));

vi.mock("@/components/dashboard/service-requirements-editor", () => ({
  ServiceRequirementsView: () => <div data-testid="requirements" />,
}));

vi.mock("sonner", () => ({
  toast: { success: mockToastSuccess, error: mockToastError },
}));

function makeNode(overrides: Partial<NodeInfo> = {}): NodeInfo {
  return {
    id: "node-1",
    name: "Edge node",
    owner: { kind: "user", id: "user-1", display_name: "User One" },
    status: "Online",
    is_connected: true,
    last_heartbeat_at: null,
    connected_at: null,
    metadata: null,
    metrics: null,
    capabilities: { remote_credential_crypto_v1: true },
    binding_count: 1,
    created_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function makeService(
  overrides: Partial<DownstreamService> = {},
): DownstreamService {
  return {
    id: "svc-1",
    name: "OpenAI",
    slug: "openai",
    description: "OpenAI API",
    base_url: "https://api.openai.com/v1",
    service_type: "api",
    visibility: "public",
    auth_method: "bearer",
    auth_type: "bearer",
    auth_key_name: "Authorization",
    is_active: true,
    oauth_client_id: null,
    api_spec_url: null,
    service_category: "ai",
    requires_user_credential: true,
    created_by: "user-1",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    identity_propagation_mode: "none",
    forward_access_token: false,
    inject_delegation_token: false,
    default_request_headers: null,
    ws_frame_injections: null,
    node_id: "node-1",
    your_user_service_id: "usvc-1",
    your_binding_count: 1,
    ...overrides,
  };
}

function expectMetadataOnlyPushBody(body: Record<string, unknown>) {
  expect(body.remote_crypto).toBe(true);
  for (const forbidden of ["secret", "credential", "token", "value"]) {
    expect(body).not.toHaveProperty(forbidden);
  }
}

beforeEach(() => {
  vi.clearAllMocks();
  hooks.node = {
    data: makeNode(),
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  };
  hooks.admins = { data: [], isLoading: false };
  hooks.pendingCredentials = { data: [], isLoading: false };
  hooks.keys = { data: [] };
  hooks.service = {
    data: makeService(),
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  };
  mockPushCredential.mockResolvedValue({ id: "pending-1" });
});

describe("credential push forms", () => {
  it("node-detail posts metadata with remote_crypto and no plaintext fields", async () => {
    const user = userEvent.setup();
    render(<NodeDetailPage />);

    await user.type(screen.getByLabelText("Service slug"), "openai");
    await user.clear(screen.getByLabelText("Target URL"));
    await user.type(
      screen.getByLabelText("Target URL"),
      "https://api.openai.com/v1",
    );
    await user.type(screen.getByLabelText("Label"), "Production OpenAI");
    await user.click(screen.getByRole("button", { name: /^push$/i }));

    await waitFor(() => expect(mockPushCredential).toHaveBeenCalledTimes(1));
    const body = mockPushCredential.mock.calls[0]![0] as Record<
      string,
      unknown
    >;
    expect(body).toMatchObject({
      service_slug: "openai",
      injection_method: "header",
      field_name: "X-API-Key",
      target_url: "https://api.openai.com/v1",
      label: "Production OpenAI",
      remote_crypto: true,
    });
    expectMetadataOnlyPushBody(body);
  });

  it("service-detail single binding with node_id posts metadata only", async () => {
    const user = userEvent.setup();
    render(<ServiceDetailPage />);

    expect(
      screen.getByRole("heading", { name: "Push credential" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^push$/i }));

    await waitFor(() => expect(mockPushCredential).toHaveBeenCalledTimes(1));
    const body = mockPushCredential.mock.calls[0]![0] as Record<
      string,
      unknown
    >;
    expect(body).toMatchObject({
      service_slug: "openai",
      injection_method: "header",
      field_name: "Authorization",
      target_url: "https://api.openai.com/v1",
      label: "OpenAI credential",
      remote_crypto: true,
    });
    expectMetadataOnlyPushBody(body);
  });

  it("service-detail with zero bindings keeps the bind CTA and hides push", () => {
    hooks.service = {
      data: makeService({
        node_id: null,
        your_user_service_id: null,
        your_binding_count: 0,
      }),
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    };

    render(<ServiceDetailPage />);

    expect(
      screen.queryByRole("heading", { name: "Push credential" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /Bind in AI Services/i }),
    ).toBeInTheDocument();
  });

  it("service-detail with multiple bindings keeps disambiguation and hides push", () => {
    hooks.service = {
      data: makeService({
        node_id: "node-1",
        your_user_service_id: null,
        your_binding_count: 2,
      }),
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    };

    render(<ServiceDetailPage />);

    expect(
      screen.queryByRole("heading", { name: "Push credential" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /Manage in AI Services/i }),
    ).toBeInTheDocument();
  });
});

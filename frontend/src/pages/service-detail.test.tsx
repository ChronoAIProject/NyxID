import type { ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DownstreamService } from "@/types/api";
import { ServiceDetailPage } from "./service-detail";

const { hooks, mockCreateAnonymousEndpoint, mockToastError, mockToastSuccess } =
  vi.hoisted(() => ({
    hooks: {
      service: {
        data: undefined as DownstreamService | undefined,
        isLoading: false,
        error: null as unknown,
        refetch: vi.fn(),
      },
      anonymousEndpoints: {
        data: [
          {
            id: "rule-1",
            enabled: true,
            method: "GET" as const,
            path_pattern: "/public/**",
            daily_quota: 25,
          },
        ],
        isLoading: false,
      },
    },
    mockCreateAnonymousEndpoint: vi.fn(),
    mockToastError: vi.fn(),
    mockToastSuccess: vi.fn(),
  }));

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
  useNavigate: () => vi.fn(),
  useParams: () => ({ serviceId: "svc-1" }),
}));

vi.mock("@/hooks/use-services", () => ({
  useService: () => hooks.service,
  useDeleteService: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useTestSshConnection: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));

vi.mock("@/hooks/use-anonymous-endpoints", () => ({
  useAnonymousEndpoints: () => hooks.anonymousEndpoints,
  useCreateAnonymousEndpoint: () => ({
    mutateAsync: mockCreateAnonymousEndpoint,
    isPending: false,
  }),
  useUpdateAnonymousEndpoint: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
  useDeleteAnonymousEndpoint: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

vi.mock("@/hooks/use-nodes", () => ({
  usePushNodeCredential: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));

vi.mock("@/hooks/use-providers", () => ({
  useMyProviderTokens: () => ({ data: [] }),
}));

vi.mock("@/hooks/use-developer-apps", () => ({
  useDeveloperApps: () => ({ data: { clients: [] } }),
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

function makeService(
  overrides: Partial<DownstreamService> = {},
): DownstreamService {
  return {
    id: "svc-1",
    name: "Public API",
    slug: "public-api",
    description: "Public API",
    base_url: "https://api.example.test",
    service_type: "http",
    visibility: "public",
    auth_method: "none",
    auth_type: "none",
    auth_key_name: "",
    is_active: true,
    oauth_client_id: null,
    api_spec_url: null,
    service_category: "connection",
    requires_user_credential: true,
    created_by: "user-1",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    identity_propagation_mode: "none",
    forward_access_token: false,
    inject_delegation_token: false,
    anonymous_endpoints: [],
    default_request_headers: null,
    ws_frame_injections: null,
    node_id: null,
    your_user_service_id: null,
    your_binding_count: 0,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  hooks.service = {
    data: makeService(),
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  };
  hooks.anonymousEndpoints = {
    data: [
      {
        id: "rule-1",
        enabled: true,
        method: "GET",
        path_pattern: "/public/**",
        daily_quota: 25,
      },
    ],
    isLoading: false,
  };
  mockCreateAnonymousEndpoint.mockResolvedValue({
    id: "rule-2",
    enabled: false,
    method: "POST",
    path_pattern: "/submit",
    daily_quota: 10,
  });
});

describe("service detail anonymous endpoints", () => {
  it("renders configured anonymous endpoint rules", () => {
    render(<ServiceDetailPage />);

    expect(
      screen.getByRole("heading", { name: "Anonymous endpoints" }),
    ).toBeInTheDocument();
    expect(screen.getAllByDisplayValue("/public/**").length).toBeGreaterThan(1);
    expect(screen.getByText("Enabled")).toBeInTheDocument();
  });

  it("creates anonymous endpoint drafts from the inline form", async () => {
    const user = userEvent.setup();
    render(<ServiceDetailPage />);

    await user.clear(screen.getByLabelText("Path"));
    await user.type(screen.getByLabelText("Path"), "/submit");
    await user.clear(screen.getByLabelText("Daily quota"));
    await user.type(screen.getByLabelText("Daily quota"), "10");
    await user.click(screen.getByRole("button", { name: /add/i }));

    await waitFor(() =>
      expect(mockCreateAnonymousEndpoint).toHaveBeenCalledWith({
        enabled: false,
        method: "GET",
        path_pattern: "/submit",
        daily_quota: 10,
      }),
    );
    expect(mockToastSuccess).toHaveBeenCalledWith("Anonymous endpoint created");
  });
});

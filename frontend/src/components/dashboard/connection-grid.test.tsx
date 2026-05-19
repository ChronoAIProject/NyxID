import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DownstreamService, UserServiceConnection } from "@/types/api";
import { ConnectionGrid } from "./connection-grid";

const mocks = vi.hoisted(() => ({
  useServices: vi.fn(),
  useConnections: vi.fn(),
  useConnectService: vi.fn(),
  useDisconnectService: vi.fn(),
  useUpdateCredential: vi.fn(),
  useMyNodeBindings: vi.fn(),
}));

vi.mock("@/hooks/use-services", () => ({
  useServices: mocks.useServices,
  useConnections: mocks.useConnections,
  useConnectService: mocks.useConnectService,
  useDisconnectService: mocks.useDisconnectService,
  useUpdateCredential: mocks.useUpdateCredential,
}));

vi.mock("@/hooks/use-nodes", () => ({
  useMyNodeBindings: mocks.useMyNodeBindings,
}));

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}));

vi.mock("./credential-dialog", () => ({
  CredentialDialog: () => null,
}));

const service: DownstreamService = {
  id: "svc-openai",
  name: "OpenAI",
  slug: "openai",
  description: "External API",
  base_url: "https://api.openai.com/v1",
  service_type: "http",
  visibility: "public",
  auth_method: "header",
  auth_type: "api_key",
  auth_key_name: "Authorization",
  is_active: true,
  oauth_client_id: null,
  openapi_spec_url: null,
  api_spec_url: null,
  asyncapi_spec_url: null,
  streaming_supported: false,
  ssh_config: null,
  service_category: "connection",
  requires_user_credential: true,
  created_by: "user-1",
  created_at: "2026-03-10T00:00:00Z",
  updated_at: "2026-03-10T00:00:00Z",
};

const brokenConnection: UserServiceConnection = {
  service_id: service.id,
  service_name: service.name,
  service_category: service.service_category,
  auth_type: service.auth_type,
  has_credential: false,
  credential_label: null,
  connected_at: "2026-03-10T00:00:00Z",
};

describe("ConnectionGrid", () => {
  beforeEach(() => {
    vi.clearAllMocks();

    mocks.useServices.mockReturnValue({
      data: [service],
      isLoading: false,
    });
    mocks.useConnections.mockReturnValue({
      data: [brokenConnection],
      isLoading: false,
    });
    mocks.useConnectService.mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    });
    mocks.useDisconnectService.mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    });
    mocks.useUpdateCredential.mockReturnValue({
      mutateAsync: vi.fn(),
      isPending: false,
    });
    mocks.useMyNodeBindings.mockReturnValue({
      data: [],
    });
  });

  it("keeps the repair action for connected services missing a web credential", () => {
    render(<ConnectionGrid />);

    expect(screen.getByText("Credential missing")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Update Key" }),
    ).toBeInTheDocument();
  });

  it("hides the repair action for node-managed credentials", () => {
    mocks.useMyNodeBindings.mockReturnValue({
      data: [service.id],
    });

    render(<ConnectionGrid />);

    expect(screen.getByText("Via node")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Update Key" }),
    ).not.toBeInTheDocument();
  });
});

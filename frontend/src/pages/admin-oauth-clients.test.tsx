import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import type {
  AdminOAuthClientListResponse,
  BrokerSettingsResponse,
} from "@/types/admin";

const {
  mockUpdateClient,
  mockUpdateSettings,
  mockUseBrokerSettings,
  clientsResponse,
  brokerSettings,
} = vi.hoisted(() => {
  const clientsResponse: AdminOAuthClientListResponse = {
    clients: [
      {
        id: "dcr-aevatar",
        client_name: "Aevatar",
        client_type: "public",
        created_by: "dynamic_registration",
        redirect_uris: ["https://aevatar.example/callback"],
        allowed_scopes: "openid urn:nyxid:scope:broker_binding",
        delegation_scopes: "",
        broker_capability_enabled: false,
        broker_capability_effective: true,
        broker_capability_source: "scope",
        revocation_webhook_url: null,
        is_active: true,
        client_secret: null,
        created_at: "2026-07-01T00:00:00Z",
      },
    ],
  };
  const brokerSettings: BrokerSettingsResponse = {
    broker_require_sender_constraint: {
      effective: true,
      env_default: false,
      override: true,
      source: "override",
    },
    broker_require_admin_capability: {
      effective: false,
      env_default: false,
      override: null,
      source: "env_default",
    },
  };
  return {
    mockUpdateClient: vi.fn(),
    mockUpdateSettings: vi.fn(),
    mockUseBrokerSettings: vi.fn(),
    clientsResponse,
    brokerSettings,
  };
});

vi.mock("@/hooks/use-admin-oauth-clients", () => ({
  useAdminOAuthClients: () => ({
    data: clientsResponse,
    isLoading: false,
    error: null,
  }),
  useBrokerSettings: mockUseBrokerSettings,
  useUpdateAdminOAuthClient: () => ({
    mutateAsync: mockUpdateClient,
    isPending: false,
  }),
  useUpdateBrokerSettings: () => ({
    mutateAsync: mockUpdateSettings,
    isPending: false,
  }),
}));

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}));

import { AdminOAuthClientsPage } from "./admin-oauth-clients";

function adminUser(): User {
  return {
    id: "admin-1",
    email: "admin@example.com",
    display_name: "Admin",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: true,
    role: "admin",
    is_active: true,
    created_at: "2026-01-01T00:00:00Z",
  };
}

function operatorUser(): User {
  return {
    ...adminUser(),
    id: "operator-1",
    email: "operator@example.com",
    display_name: "Operator",
    is_admin: false,
    is_operator: true,
    role: "operator",
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockUpdateClient.mockResolvedValue(clientsResponse.clients[0]);
  mockUpdateSettings.mockResolvedValue(brokerSettings);
  mockUseBrokerSettings.mockReturnValue({
    data: brokerSettings,
    isLoading: false,
    error: null,
  });
  useAuthStore.setState({
    user: adminUser(),
    isAuthenticated: true,
    isLoading: false,
  });
});

describe("AdminOAuthClientsPage", () => {
  it("renders all clients including dynamic-registration clients", () => {
    render(<AdminOAuthClientsPage />);

    expect(screen.getByText("Aevatar")).toBeInTheDocument();
    expect(screen.getByText("dcr-aevatar")).toBeInTheDocument();
    expect(screen.getByText("dynamic_registration")).toBeInTheDocument();
    expect(
      screen.getByText("urn:nyxid:scope:broker_binding"),
    ).toBeInTheDocument();
  });

  it("confirms and disables effective broker capability by removing the legacy scope trigger", async () => {
    const user = userEvent.setup();
    render(<AdminOAuthClientsPage />);

    await user.click(
      screen.getByRole("switch", {
        name: "Toggle broker capability for Aevatar",
      }),
    );

    expect(
      screen.getByText(/removes the legacy broker-binding scope/i),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Disable capability" }),
    );

    expect(mockUpdateClient).toHaveBeenCalledWith({
      clientId: "dcr-aevatar",
      data: {
        broker_capability_enabled: false,
        allowed_scopes: ["openid"],
      },
    });
  });

  it("shows every write control and the broker policy card to admins", () => {
    render(<AdminOAuthClientsPage />);

    expect(screen.getByText("Broker Rollout Policy")).toBeInTheDocument();
    expect(
      screen.getByRole("switch", {
        name: "Toggle broker capability for Aevatar",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", {
        name: "Toggle active status for Aevatar",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", {
        name: "Toggle Require sender constraint",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", {
        name: "Toggle Require admin broker capability",
      }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reset" })).toBeInTheDocument();
    expect(screen.getByText("Overridden")).toBeInTheDocument();
    expect(screen.getByText("Env default")).toBeInTheDocument();
    expect(screen.getByText("Scope")).toBeInTheDocument();
    expect(screen.getAllByText("Enabled").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Disabled").length).toBeGreaterThan(0);
    expect(mockUseBrokerSettings).toHaveBeenCalledWith(true);
  });

  it("shows operators a read-only client list without broker policy controls", () => {
    useAuthStore.setState({ user: operatorUser() });

    render(<AdminOAuthClientsPage />);

    expect(screen.getByText("Aevatar")).toBeInTheDocument();
    expect(screen.getByText("Scope")).toBeInTheDocument();
    expect(screen.queryByText("Broker Rollout Policy")).not.toBeInTheDocument();
    expect(screen.queryAllByRole("switch")).toHaveLength(0);
    expect(
      screen.queryByRole("button", { name: "Reset" }),
    ).not.toBeInTheDocument();
    expect(mockUseBrokerSettings).toHaveBeenCalledWith(false);
  });

  it("resets an overridden broker setting to the env default", async () => {
    const user = userEvent.setup();
    render(<AdminOAuthClientsPage />);

    await user.click(screen.getByRole("button", { name: "Reset" }));
    await user.click(
      screen.getByRole("button", { name: "Reset to env default" }),
    );

    expect(mockUpdateSettings).toHaveBeenCalledWith({
      broker_require_sender_constraint: null,
    });
  });
});

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OAuthClient } from "@/types/api";

const mocks = vi.hoisted(() => ({
  createMutateAsync: vi.fn(),
  navigate: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  useDeveloperApps: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mocks.navigate,
}));

vi.mock("@/hooks/use-developer-apps", () => ({
  useCreateDeveloperApp: () => ({
    mutateAsync: mocks.createMutateAsync,
    isPending: false,
  }),
  useDeveloperApps: mocks.useDeveloperApps,
}));

vi.mock("sonner", () => ({
  toast: {
    error: mocks.toastError,
    success: mocks.toastSuccess,
  },
}));

import { OrgDeveloperAppsTab } from "./org-developer-apps-tab";

const oauthClient: OAuthClient = {
  id: "client-1",
  client_name: "Acme OAuth",
  client_type: "confidential",
  redirect_uris: ["https://app.example.com/callback"],
  allowed_scopes: "openid profile email",
  delegation_scopes: "",
  broker_capability_enabled: false,
  is_active: true,
  client_secret: null,
  created_at: "2026-04-20T00:00:00Z",
};

describe("OrgDeveloperAppsTab", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.useDeveloperApps.mockReturnValue({
      data: { clients: [] },
      isLoading: false,
      error: null,
    });
    mocks.createMutateAsync.mockResolvedValue({
      ...oauthClient,
      id: "client-2",
      client_name: "Portal App",
      client_type: "public",
    });
  });

  it("renders an org-owned empty state", () => {
    render(<OrgDeveloperAppsTab orgId="org-1" orgName="Acme Org" />);

    expect(
      screen.getByText("No developer apps owned by Acme Org."),
    ).toBeInTheDocument();
    expect(mocks.useDeveloperApps).toHaveBeenCalledWith("org-1");
  });

  it("navigates to the org-scoped detail route from a card", async () => {
    const user = userEvent.setup();
    mocks.useDeveloperApps.mockReturnValue({
      data: { clients: [oauthClient] },
      isLoading: false,
      error: null,
    });

    render(<OrgDeveloperAppsTab orgId="org-1" orgName="Acme Org" />);
    await user.click(screen.getByText("Acme OAuth"));

    expect(mocks.navigate).toHaveBeenCalledWith({
      to: "/orgs/$orgId/developer-apps/$clientId",
      params: { orgId: "org-1", clientId: "client-1" },
    });
  });

  it("injects target_org_id when creating a developer app", async () => {
    const user = userEvent.setup();

    render(<OrgDeveloperAppsTab orgId="org-1" orgName="Acme Org" />);
    await user.click(screen.getByRole("button", { name: /new application/i }));

    expect(screen.getByText("Organization")).toBeInTheDocument();
    expect(screen.getByText("Acme Org")).toBeInTheDocument();

    await user.type(screen.getByLabelText("Application Name"), "Portal App");
    await user.type(
      screen.getByLabelText("Redirect URIs (one per line)"),
      "https://portal.example.com/callback",
    );
    await user.click(screen.getByRole("button", { name: /create app/i }));

    await waitFor(() => {
      expect(mocks.createMutateAsync).toHaveBeenCalledWith({
        name: "Portal App",
        redirect_uris: ["https://portal.example.com/callback"],
        client_type: "public",
        allowed_scopes: ["openid", "profile", "email"],
        target_org_id: "org-1",
      });
    });
  });
});

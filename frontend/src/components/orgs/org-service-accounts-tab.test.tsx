import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ServiceAccountListResponse } from "@/types/service-accounts";

const mocks = vi.hoisted(() => ({
  createMutateAsync: vi.fn(),
  navigate: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  useServiceAccounts: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mocks.navigate,
}));

vi.mock("@/hooks/use-service-accounts", () => ({
  useCreateServiceAccount: () => ({
    mutateAsync: mocks.createMutateAsync,
    isPending: false,
  }),
  useServiceAccounts: mocks.useServiceAccounts,
}));

vi.mock("sonner", () => ({
  toast: {
    error: mocks.toastError,
    success: mocks.toastSuccess,
  },
}));

import { OrgServiceAccountsTab } from "./org-service-accounts-tab";

const emptyList: ServiceAccountListResponse = {
  service_accounts: [],
  total: 0,
  page: 1,
  per_page: 20,
};

const listWithAccount: ServiceAccountListResponse = {
  service_accounts: [
    {
      id: "sa-1",
      name: "CI Bot",
      description: null,
      client_id: "nyx_sa_ci",
      secret_prefix: "nyx_ssec",
      allowed_scopes: "openid proxy:*",
      role_ids: [],
      is_active: true,
      rate_limit_override: null,
      created_by: "user-1",
      created_at: "2026-04-20T00:00:00Z",
      updated_at: "2026-04-20T00:00:00Z",
      last_authenticated_at: null,
    },
  ],
  total: 1,
  page: 1,
  per_page: 20,
};

describe("OrgServiceAccountsTab", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.useServiceAccounts.mockReturnValue({
      data: emptyList,
      isLoading: false,
      error: null,
    });
    mocks.createMutateAsync.mockResolvedValue({
      id: "sa-2",
      name: "Deploy Bot",
      client_id: "nyx_sa_deploy",
      client_secret: "secret-once",
      allowed_scopes: "openid",
      role_ids: [],
      is_active: true,
      created_at: "2026-04-20T00:00:00Z",
      message: "created",
    });
  });

  it("renders an org-owned empty state", () => {
    render(<OrgServiceAccountsTab orgId="org-1" orgName="Acme Org" />);

    expect(
      screen.getByText("No service accounts owned by Acme Org."),
    ).toBeInTheDocument();
    expect(mocks.useServiceAccounts).toHaveBeenCalledWith(1, 20, "", "org-1");
  });

  it("navigates to the org-scoped detail route from a row", async () => {
    const user = userEvent.setup();
    mocks.useServiceAccounts.mockReturnValue({
      data: listWithAccount,
      isLoading: false,
      error: null,
    });

    render(<OrgServiceAccountsTab orgId="org-1" orgName="Acme Org" />);
    await user.click(screen.getByText("CI Bot"));

    expect(mocks.navigate).toHaveBeenCalledWith({
      to: "/orgs/$orgId/service-accounts/$saId",
      params: { orgId: "org-1", saId: "sa-1" },
    });
  });

  it("injects target_org_id when creating a service account", async () => {
    const user = userEvent.setup();

    render(<OrgServiceAccountsTab orgId="org-1" orgName="Acme Org" />);
    await user.click(
      screen.getByRole("button", { name: /create service account/i }),
    );

    expect(screen.getByText("Organization")).toBeInTheDocument();
    expect(screen.getByText("Acme Org")).toBeInTheDocument();

    await user.type(screen.getByLabelText("Name"), "Deploy Bot");
    await user.type(screen.getByLabelText("Allowed Scopes"), "openid");
    await user.click(screen.getByRole("button", { name: /^Create$/i }));

    await waitFor(() => {
      expect(mocks.createMutateAsync).toHaveBeenCalledWith({
        name: "Deploy Bot",
        description: undefined,
        allowed_scopes: "openid",
        role_ids: undefined,
        rate_limit_override: undefined,
        target_org_id: "org-1",
      });
    });
  });
});

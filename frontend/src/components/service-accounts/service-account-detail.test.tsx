import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { Role } from "@/types/rbac";
import { ServiceAccountDetail } from "./service-account-detail";
const mock = vi.hoisted(() => ({
  update: vi.fn(),
  isAdmin: false,
  roles: [] as Role[],
  rolesError: false,
  rolesQuery: vi.fn(),
  account: {
    id: "sa-1",
    name: "Worker",
    description: "Saved",
    client_id: "client-1",
    secret_prefix: "prefix",
    allowed_scopes: "openid",
    role_ids: ["role-a"],
    purpose: "general" as "general" | "catalog_editor",
    rate_limit_override: 10,
    is_active: true,
    created_at: "2026-09-01T00:00:00Z",
    updated_at: "2026-09-01T00:00:00Z",
    last_authenticated_at: null,
  },
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (state: { user: { is_admin: boolean } }) => unknown) =>
    select({ user: { is_admin: mock.isAdmin } }),
}));
vi.mock("@/hooks/use-rbac", () => ({
  useRoles: (options: { enabled: boolean }) => {
    mock.rolesQuery(options);
    return {
      data: { roles: mock.roles },
      isError: mock.rolesError,
      refetch: vi.fn(),
    };
  },
}));
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  useSearch: () => ({}),
}));
vi.mock("@/hooks/use-service-accounts", () => ({
  useServiceAccount: () => ({ data: mock.account, isLoading: false }),
  useUpdateServiceAccount: () => ({
    mutateAsync: mock.update,
    isPending: false,
  }),
  useDeleteServiceAccount: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useRotateSecret: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useRevokeTokens: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));
vi.mock("@/components/dashboard/sa-connected-services", () => ({
  SaConnectedServices: () => <div data-testid="provider-connections" />,
}));
vi.mock("./curation-grant-section", () => ({
  CurationGrantSection: () => <div data-testid="curation-grant-section" />,
}));
vi.mock("./key-read-grant-section", () => ({
  KeyReadGrantSection: ({ saId }: { readonly saId: string }) => (
    <div data-testid="key-read-grant-section">{saId}</div>
  ),
}));
vi.mock("@/hooks/use-options", () => ({
  useOptions: () => ({
    data: { pages: [{ items: [] }] },
    hasNextPage: false,
    isFetching: false,
    isFetchingNextPage: false,
    isError: false,
    error: null,
    fetchNextPage: vi.fn(),
    retainPartialData: false,
  }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  mock.isAdmin = false;
  mock.roles = [];
  mock.rolesError = false;
  mock.account = {
    ...mock.account,
    id: "sa-1",
    name: "Worker",
    role_ids: ["role-a"],
    purpose: "general",
    allowed_scopes: "openid",
    is_active: true,
  };
});

function catalogRole(): Role {
  return {
    id: "role-a",
    name: "Catalog editor",
    slug: "catalog-editor",
    description: null,
    client_id: null,
    permissions: ["nyxid:catalog:skills:read", "nyxid:catalog:skills:write"],
    is_default: false,
    is_system: false,
    created_at: "2026-09-01T00:00:00Z",
    updated_at: "2026-09-01T00:00:00Z",
  };
}

it("keeps legacy grants manageable until catalog activation succeeds", () => {
  mock.isAdmin = true;
  mock.roles = [catalogRole()];
  mock.account.allowed_scopes =
    "catalog:skills:read catalog:skills:write user-services:read proxy";
  render(
    <ServiceAccountDetail
      saId="sa-1"
      backTo={{ to: "/admin/service-accounts", label: "Accounts" }}
    />,
  );

  expect(
    screen.getByRole("button", { name: "Apply catalog access" }),
  ).toBeEnabled();
  expect(screen.queryByTestId("key-read-grant-section")).toBeInTheDocument();
  expect(screen.queryByTestId("curation-grant-section")).toBeInTheDocument();
  expect(screen.getByTestId("provider-connections")).toBeInTheDocument();
});

it("does not treat scope strings or client-associated roles as catalog authority", () => {
  mock.isAdmin = true;
  mock.roles = [{ ...catalogRole(), client_id: "oauth-client" }];
  mock.account.allowed_scopes =
    "catalog:skills:read catalog:skills:write user-services:read proxy";
  render(
    <ServiceAccountDetail
      saId="sa-1"
      backTo={{ to: "/admin/service-accounts", label: "Accounts" }}
    />,
  );

  expect(screen.getByTestId("key-read-grant-section")).toBeInTheDocument();
  expect(screen.getByTestId("curation-grant-section")).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Apply catalog access" }),
  ).not.toBeEnabled();
});

it("does not hide grants or offer activation when role checking fails", () => {
  mock.isAdmin = true;
  mock.roles = [catalogRole()];
  mock.rolesError = true;
  render(
    <ServiceAccountDetail
      saId="sa-1"
      backTo={{ to: "/admin/service-accounts", label: "Accounts" }}
    />,
  );

  expect(screen.getByRole("alert")).toHaveTextContent("Could not check");
  expect(screen.getByTestId("key-read-grant-section")).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Apply catalog access" }),
  ).not.toBeInTheDocument();
});

it("keeps key read grants available when provider sections are hidden", () => {
  render(
    <ServiceAccountDetail
      saId="sa-1"
      backTo={{ to: "/orgs/org-1", label: "Organization" }}
      showProviderSections={false}
    />,
  );

  expect(screen.getByTestId("key-read-grant-section")).toHaveTextContent(
    "sa-1",
  );
  expect(mock.rolesQuery).toHaveBeenCalledWith({ enabled: false });
});

it("can hide key read grants independently of provider sections", () => {
  render(
    <ServiceAccountDetail
      saId="sa-1"
      backTo={{ to: "/orgs/org-1", label: "Organization" }}
      showProviderSections={false}
      showKeyReadGrantSection={false}
    />,
  );

  expect(
    screen.queryByTestId("key-read-grant-section"),
  ).not.toBeInTheDocument();
});

it("does not restore revoked permissions after a refresh when renaming an account", async () => {
  const user = userEvent.setup();
  const element = (
    <ServiceAccountDetail
      saId="sa-1"
      backTo={{ to: "/admin/service-accounts", label: "Accounts" }}
      showProviderSections={false}
    />
  );
  const view = render(element);
  await user.click(screen.getByRole("button", { name: "Edit" }));
  mock.account = { ...mock.account, is_active: false, role_ids: [] };
  view.rerender(
    <ServiceAccountDetail
      saId="sa-1"
      backTo={{ to: "/admin/service-accounts", label: "Accounts" }}
      showProviderSections={false}
    />,
  );
  fireEvent.change(screen.getByLabelText("Name"), {
    target: { value: "Renamed worker" },
  });
  await user.click(screen.getByRole("button", { name: "Save Changes" }));
  expect(mock.update).not.toHaveBeenCalled();
  await user.click(
    await screen.findByRole("button", { name: "Confirm changes" }),
  );
  await waitFor(() =>
    expect(mock.update).toHaveBeenCalledExactlyOnceWith({
      saId: "sa-1",
      data: { name: "Renamed worker" },
    }),
  );
});

it("cancels A's pending review when the source becomes B", async () => {
  mock.update.mockClear();
  const user = userEvent.setup();
  const props = {
    backTo: { to: "/admin/service-accounts", label: "Accounts" },
    showProviderSections: false,
  };
  const view = render(<ServiceAccountDetail saId="sa-1" {...props} />);
  await user.click(screen.getByRole("button", { name: "Edit" }));
  fireEvent.change(screen.getByLabelText("Name"), {
    target: { value: "A draft" },
  });
  await user.click(screen.getByRole("button", { name: "Save Changes" }));
  await screen.findByRole("button", { name: "Confirm changes" });
  mock.account = { ...mock.account, id: "sa-2", name: "B" };
  view.rerender(<ServiceAccountDetail saId="sa-2" {...props} />);
  expect(
    screen.queryByRole("button", { name: "Confirm changes" }),
  ).not.toBeInTheDocument();
  expect(mock.update).not.toHaveBeenCalled();
});

it("blocks a reviewed role replacement after an observed revocation", async () => {
  mock.update.mockClear();
  mock.account = { ...mock.account, id: "sa-1", role_ids: ["role-a"] };
  const user = userEvent.setup();
  const props = {
    saId: "sa-1",
    backTo: { to: "/admin/service-accounts", label: "Accounts" },
    showProviderSections: false,
  };
  const view = render(<ServiceAccountDetail {...props} />);
  await user.click(screen.getByRole("button", { name: "Edit" }));
  fireEvent.change(screen.getByLabelText("Role IDs (comma-separated)"), {
    target: { value: "role-a, role-b" },
  });
  await user.click(screen.getByRole("button", { name: "Save Changes" }));
  await screen.findByRole("button", { name: "Confirm changes" });
  mock.account = { ...mock.account, role_ids: [] };
  view.rerender(<ServiceAccountDetail {...props} />);
  expect(
    screen.getByRole("button", { name: "Confirm changes" }),
  ).toBeDisabled();
  expect(mock.update).not.toHaveBeenCalled();
});

it("shows standing catalog authority without UUID grant forms and keeps provider connections", () => {
  mock.account = {
    ...mock.account,
    purpose: "catalog_editor",
    allowed_scopes:
      "catalog:skills:read catalog:skills:write user-services:read proxy",
    role_ids: ["catalog-editor-role"],
  };
  render(
    <ServiceAccountDetail
      saId="sa-1"
      backTo={{ to: "/admin/service-accounts", label: "Accounts" }}
    />,
  );

  expect(
    screen.getByText("All current and future catalog services"),
  ).toBeInTheDocument();
  expect(
    screen.getByText(
      /role must retain the matching NyxID catalog permissions/i,
    ),
  ).toBeInTheDocument();
  expect(
    screen.queryByTestId("curation-grant-section"),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByTestId("key-read-grant-section"),
  ).not.toBeInTheDocument();
  expect(screen.getByTestId("provider-connections")).toBeInTheDocument();
});

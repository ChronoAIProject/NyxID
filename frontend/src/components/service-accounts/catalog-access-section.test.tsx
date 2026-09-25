import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { CatalogAccessSection } from "./catalog-access-section";
import type { ServiceAccount } from "@/types/service-accounts";
import type { Role } from "@/types/rbac";
const state = vi.hoisted(() => ({ save: vi.fn() }));
vi.mock("@/hooks/use-service-accounts", () => ({
  useUpdateServiceAccount: () => ({ mutateAsync: state.save }),
}));
const account: ServiceAccount = {
  id: "sa",
  name: "Aevatar",
  description: null,
  client_id: "client",
  secret_prefix: "prefix",
  allowed_scopes: "proxy",
  role_ids: ["editor", "ornn"],
  is_active: true,
  rate_limit_override: null,
  created_by: "admin",
  created_at: "",
  updated_at: "",
  last_authenticated_at: null,
  purpose: "general",
};
const role: Role = {
  id: "editor",
  name: "Catalog editor",
  slug: "editor",
  description: null,
  permissions: ["nyxid:catalog:skills:read", "nyxid:catalog:skills:write"],
  client_id: null,
  is_default: false,
  is_system: false,
  created_at: "",
  updated_at: "",
};
beforeEach(() => {
  state.save.mockReset();
  state.save.mockResolvedValue({
    ...account,
    purpose: "catalog_editor",
    platform_protected: true,
  });
});
it("applies unchanged roles and adds required scopes while preserving proxy and disabled state", async () => {
  const user = userEvent.setup();
  render(
    <CatalogAccessSection
      account={{ ...account, is_active: false }}
      roles={[role]}
      onEditAccount={vi.fn()}
    />,
  );
  await user.click(
    screen.getByRole("button", { name: "Apply catalog access" }),
  );
  expect(
    screen.getByText(/managed by platform administrators/),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Confirm changes" }));
  await waitFor(() =>
    expect(state.save).toHaveBeenCalledExactlyOnceWith({
      saId: "sa",
      data: {
        expected_access: {
          role_ids: account.role_ids,
          allowed_scopes: "proxy",
          purpose: "general",
          platform_protected: false,
          is_active: false,
        },
        role_ids: ["editor", "ornn"],
        allowed_scopes:
          "proxy catalog:skills:read user-services:read catalog:skills:write",
      },
    }),
  );
});
it("repairs missing scopes on an existing editor", async () => {
  const user = userEvent.setup();
  render(
    <CatalogAccessSection
      account={{
        ...account,
        purpose: "catalog_editor",
        platform_protected: true,
      }}
      roles={[role]}
      onEditAccount={vi.fn()}
    />,
  );
  await user.click(
    screen.getByRole("button", { name: "Apply catalog access" }),
  );
  await user.click(screen.getByRole("button", { name: "Confirm changes" }));
  await waitFor(() => expect(state.save).toHaveBeenCalledOnce());
});

it("activates an existing account even when all roles and scopes are unchanged", async () => {
  const user = userEvent.setup();
  const configured = {
    ...account,
    allowed_scopes:
      "catalog:skills:read catalog:skills:write user-services:read proxy",
  };
  render(
    <CatalogAccessSection
      account={configured}
      roles={[role]}
      onEditAccount={vi.fn()}
    />,
  );
  await user.click(
    screen.getByRole("button", { name: "Apply catalog access" }),
  );
  expect(screen.getByText("Catalog access and management")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Confirm changes" }));
  await waitFor(() =>
    expect(state.save).toHaveBeenCalledExactlyOnceWith({
      saId: account.id,
      data: {
        role_ids: account.role_ids,
        allowed_scopes: configured.allowed_scopes,
        expected_access: {
          role_ids: account.role_ids,
          allowed_scopes: configured.allowed_scopes,
          purpose: "general",
          platform_protected: false,
          is_active: true,
        },
      },
    }),
  );
});

it.each([
  { permissions: ["nyxid:catalog:skills:write"], active: true },
  { permissions: [], active: true },
  { permissions: role.permissions, active: false },
])(
  "does not report catalog reads ready without live read authority",
  ({ permissions, active }) => {
    render(
      <CatalogAccessSection
        account={{
          ...account,
          purpose: "catalog_editor",
          platform_protected: true,
          is_active: active,
          allowed_scopes:
            "catalog:skills:read catalog:skills:write user-services:read proxy",
        }}
        roles={[{ ...role, permissions }]}
        onEditAccount={vi.fn()}
      />,
    );
    expect(screen.getByText(/Catalog read settings:/)).toHaveTextContent(
      "Catalog read settings: Incomplete",
    );
  },
);

it("adds read scopes without granting write scope for a read-only role", async () => {
  const user = userEvent.setup();
  render(
    <CatalogAccessSection
      account={account}
      roles={[{ ...role, permissions: ["nyxid:catalog:skills:read"] }]}
      onEditAccount={vi.fn()}
    />,
  );
  await user.click(
    screen.getByRole("button", { name: "Apply catalog access" }),
  );
  await user.click(screen.getByRole("button", { name: "Confirm changes" }));
  await waitFor(() =>
    expect(state.save).toHaveBeenCalledExactlyOnceWith({
      saId: account.id,
      data: {
        role_ids: account.role_ids,
        allowed_scopes: "proxy catalog:skills:read user-services:read",
        expected_access: {
          role_ids: account.role_ids,
          allowed_scopes: account.allowed_scopes,
          purpose: "general",
          platform_protected: false,
          is_active: true,
        },
      },
    }),
  );
});
it.each([
  { ...role, client_id: "client" },
  { ...role, permissions: ["*"] },
])("does not activate from client-specific or wildcard roles", (invalid) => {
  render(
    <CatalogAccessSection
      account={account}
      roles={[invalid]}
      onEditAccount={vi.fn()}
    />,
  );
  expect(
    screen.getByRole("button", { name: "Apply catalog access" }),
  ).toBeDisabled();
});
it.each(["role", "scope"])(
  "blocks confirmation after observed %s changes",
  async (kind) => {
    const user = userEvent.setup();
    const view = render(
      <CatalogAccessSection
        account={account}
        roles={[role]}
        onEditAccount={vi.fn()}
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "Apply catalog access" }),
    );
    view.rerender(
      <CatalogAccessSection
        account={
          kind === "scope"
            ? { ...account, allowed_scopes: "catalog:skills:read" }
            : account
        }
        roles={kind === "role" ? [{ ...role, permissions: [] }] : [role]}
        onEditAccount={vi.fn()}
      />,
    );
    expect(
      screen.getByRole("button", { name: "Confirm changes" }),
    ).toBeDisabled();
    expect(state.save).not.toHaveBeenCalled();
  },
);
it("reports a server response that did not activate catalog editing", async () => {
  state.save.mockResolvedValue(account);
  const user = userEvent.setup();
  render(
    <CatalogAccessSection
      account={account}
      roles={[role]}
      onEditAccount={vi.fn()}
    />,
  );
  await user.click(
    screen.getByRole("button", { name: "Apply catalog access" }),
  );
  await user.click(screen.getByRole("button", { name: "Confirm changes" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Catalog access was not activated",
  );
});
it("requires explicit editing of unsupported scopes", () => {
  render(
    <CatalogAccessSection
      account={{ ...account, allowed_scopes: "proxy roles" }}
      roles={[role]}
      onEditAccount={vi.fn()}
    />,
  );
  expect(
    screen.getByRole("button", { name: "Apply catalog access" }),
  ).toBeDisabled();
  expect(screen.getByRole("alert")).toHaveTextContent("roles");
});

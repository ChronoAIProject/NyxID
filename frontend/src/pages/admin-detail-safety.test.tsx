import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { AdminRoleDetailPage } from "./admin-role-detail";
import { AdminGroupDetailPage } from "./admin-group-detail";
import { AdminUserDetailPage } from "./admin-user-detail";

const mock = vi.hoisted(() => ({
  id: "entity-a",
  update: vi.fn(),
  role: {
    id: "entity-a",
    name: "Role A",
    slug: "role-a",
    description: "Saved",
    permissions: ["read"],
    is_default: false,
    is_system: false,
    created_at: "2026-09-01",
    updated_at: "2026-09-01",
  },
  group: {
    id: "entity-a",
    name: "Group A",
    slug: "group-a",
    description: "Saved",
    roles: [{ id: "role-a", name: "Role A", slug: "role-a" }],
    parent_group_id: null,
    is_system: false,
    created_at: "2026-09-01",
    updated_at: "2026-09-01",
  },
  user: {
    id: "entity-a",
    display_name: "User A",
    email: "a@example.com",
    avatar_url: null,
    is_admin: false,
    is_operator: false,
    is_active: true,
    email_verified: true,
    created_at: "2026-09-01",
    updated_at: "2026-09-01",
  },
}));
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ roleId: mock.id, groupId: mock.id, userId: mock.id }),
  useNavigate: () => vi.fn(),
}));
vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: () => {},
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (value: unknown) => unknown) =>
    select({ user: { id: "admin", is_admin: true } }),
}));
vi.mock("@/hooks/use-rbac", () => {
  const mutation = () => ({ mutateAsync: mock.update, isPending: false });
  return {
    useRole: () => ({ data: mock.role, isLoading: false }),
    useGroup: () => ({ data: mock.group, isLoading: false }),
    useRoles: () => ({
      data: {
        roles: [
          { id: "role-a", name: "Role A", slug: "role-a" },
          { id: "role-b", name: "Role B", slug: "role-b" },
        ],
      },
    }),
    useGroupMembers: () => ({ data: { members: [] } }),
    useUserRoles: () => ({ data: { roles: [], effective_permissions: [] } }),
    useUserGroups: () => ({ data: { groups: [] } }),
    useUpdateRole: mutation,
    useUpdateGroup: mutation,
    useDeleteRole: mutation,
    useBulkAssignRole: mutation,
    useDeleteGroup: mutation,
    useAddGroupMember: mutation,
    useRemoveGroupMember: mutation,
    useAssignRole: mutation,
    useRevokeRole: mutation,
  };
});
vi.mock("@/hooks/use-admin", () => {
  const mutation = () => ({ mutateAsync: mock.update, isPending: false });
  return {
    useAdminUser: () => ({ data: mock.user, isLoading: false }),
    useAdminUserSessions: () => ({ data: { sessions: [] } }),
    useUpdateAdminUser: mutation,
    useSetUserRole: mutation,
    useSetUserStatus: mutation,
    useForcePasswordReset: mutation,
    useDeleteUser: mutation,
    useVerifyUserEmail: mutation,
    useRevokeUserSessions: mutation,
  };
});
beforeEach(() => {
  mock.id = "entity-a";
  mock.update.mockReset().mockResolvedValue({});
  mock.role.permissions = ["read"];
  mock.group.roles = [{ id: "role-a", name: "Role A", slug: "role-a" }];
});

it.each([
  ["role", AdminRoleDetailPage, "Name"],
  ["group", AdminGroupDetailPage, "Name"],
  ["user", AdminUserDetailPage, "Display Name"],
] as const)(
  "cancels a %s review on a route identity switch",
  async (_, Component, label) => {
    const user = userEvent.setup();
    const view = render(<Component />);
    await user.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText(label), {
      target: { value: "A draft" },
    });
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    await screen.findByRole("button", { name: "Confirm changes" });
    mock.id = "entity-b";
    view.rerender(<Component />);
    expect(
      screen.queryByRole("button", { name: "Confirm changes" }),
    ).not.toBeInTheDocument();
    expect(mock.update).not.toHaveBeenCalled();
  },
);

it.each([
  ["role", AdminRoleDetailPage, "Permissions", "read, write"],
  ["group", AdminGroupDetailPage, "Roles", "role-a, role-b"],
] as const)(
  "blocks an observed %s authorization conflict but allows an unrelated rename",
  async (kind, Component, label, value) => {
    const user = userEvent.setup();
    const view = render(<Component />);
    await user.click(screen.getByRole("button", { name: "Edit" }));
    if (kind === "group")
      await user.selectOptions(screen.getByLabelText(label), [
        "role-a",
        "role-b",
      ]);
    else fireEvent.change(screen.getByLabelText(label), { target: { value } });
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    await screen.findByRole("button", { name: "Confirm changes" });
    if (kind === "role") mock.role = { ...mock.role, permissions: [] };
    else mock.group = { ...mock.group, roles: [] };
    view.rerender(<Component />);
    expect(
      screen.getByRole("button", { name: "Confirm changes" }),
    ).toBeDisabled();
    expect(mock.update).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    if (kind === "group")
      await user.deselectOptions(screen.getByLabelText(label), "role-b");
    else
      fireEvent.change(screen.getByLabelText(label), {
        target: { value: "read" },
      });
    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "Rename only" },
    });
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    await user.click(
      await screen.findByRole("button", { name: "Confirm changes" }),
    );
    await waitFor(() =>
      expect(mock.update).toHaveBeenCalledWith({
        [kind === "role" ? "roleId" : "groupId"]: "entity-a",
        data: { name: "Rename only" },
      }),
    );
  },
);

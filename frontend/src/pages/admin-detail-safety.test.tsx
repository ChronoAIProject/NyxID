import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
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
  ["role", AdminRoleDetailPage],
  ["group", AdminGroupDetailPage],
] as const)(
  "allows a name-only %s save when authorization is revoked concurrently",
  async (kind, Component) => {
    const user = userEvent.setup();
    const view = render(<Component />);
    await user.click(screen.getByRole("button", { name: "Edit" }));
    const editor = await screen.findByRole("dialog", {
      name: kind === "group" ? "Edit Group" : "Edit Role",
    });
    fireEvent.change(within(editor).getByLabelText("Name"), {
      target: { value: "Rename only" },
    });

    if (kind === "role") mock.role = { ...mock.role, permissions: [] };
    else mock.group = { ...mock.group, roles: [] };
    view.rerender(<Component />);

    await user.click(
      within(editor).getByRole("button", { name: "Save Changes" }),
    );
    const review = await screen.findByRole("dialog", {
      name: "Review changes",
    });
    const confirm = within(review).getByRole("button", {
      name: "Confirm changes",
    });
    expect(confirm).toBeEnabled();
    expect(mock.update).not.toHaveBeenCalled();
    await user.click(confirm);
    await waitFor(() =>
      expect(mock.update).toHaveBeenCalledExactlyOnceWith({
        [kind === "role" ? "roleId" : "groupId"]: "entity-a",
        data: { name: "Rename only" },
      }),
    );
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
    const editor = await screen.findByRole("dialog", {
      name: kind === "group" ? "Edit Group" : "Edit Role",
    });
    const authorizationInput = within(editor).getByLabelText(label);
    if (kind === "group") {
      expect(authorizationInput).toHaveValue(["role-a"]);
      await user.selectOptions(authorizationInput, "role-b");
      // Finish the native select interaction before submitting the form.
      await user.tab();
    } else fireEvent.change(authorizationInput, { target: { value } });
    await waitFor(() =>
      expect(authorizationInput).toHaveValue(
        kind === "group" ? ["role-a", "role-b"] : value,
      ),
    );
    expect(within(editor).getByLabelText("Name")).toHaveValue(
      kind === "group" ? "Group A" : "Role A",
    );
    expect(authorizationInput.closest("form")).toBeValid();
    const save = within(editor).getByRole("button", { name: "Save Changes" });
    expect(save).toBeEnabled();
    await user.click(save);
    const review = await screen.findByRole("dialog", { name: "Review changes" });
    const confirm = within(review).getByRole("button", {
      name: "Confirm changes",
    });
    expect(confirm).toBeEnabled();
    expect(
      within(review).getByText(kind === "group" ? /role-b/ : /write/),
    ).toBeInTheDocument();
    if (kind === "role") mock.role = { ...mock.role, permissions: [] };
    else mock.group = { ...mock.group, roles: [] };
    view.rerender(<Component />);
    expect(confirm).toBeDisabled();
    expect(within(review).getByRole("alert")).toHaveTextContent(
      "Saved values changed while you were editing",
    );
    await user.click(confirm);
    expect(mock.update).not.toHaveBeenCalled();
    await user.click(within(review).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(review).not.toBeInTheDocument());
    // Reload the editor from the observed record before the unrelated rename.
    // This mirrors a supported conflict-resolution path and avoids relying
    // on happy-dom's controlled multiple-select deselection after a nested
    // portal closes; the real Chromium flow covers that native interaction.
    await user.click(
      within(editor).getByRole("button", { name: "Cancel" }),
    );
    await waitFor(() => expect(editor).not.toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Edit" }));
    const reopenedEditor = await screen.findByRole("dialog", {
      name: kind === "group" ? "Edit Group" : "Edit Role",
    });
    fireEvent.change(within(reopenedEditor).getByLabelText("Name"), {
      target: { value: "Rename only" },
    });
    await user.click(
      within(reopenedEditor).getByRole("button", { name: "Save Changes" }),
    );
    const renameReview = await screen.findByRole("dialog", {
      name: "Review changes",
    });
    expect(
      within(renameReview).getByRole("button", { name: "Confirm changes" }),
    ).toBeEnabled();
    await user.click(
      within(renameReview).getByRole("button", { name: "Confirm changes" }),
    );
    await waitFor(() =>
      expect(mock.update).toHaveBeenCalledExactlyOnceWith({
        [kind === "role" ? "roleId" : "groupId"]: "entity-a",
        data: { name: "Rename only" },
      }),
    );
    await waitFor(() =>
      expect(screen.queryAllByRole("dialog", { hidden: true })).toHaveLength(0),
    );
  },
);

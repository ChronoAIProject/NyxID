import type { ReactNode } from "react";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSyncExternalStore } from "react";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { MemberResponse, OrgRole } from "@/schemas/orgs";
import type { OrgRoleScope } from "@/schemas/org-role-scopes";
import type { KeyInfo } from "@/types/keys";
import { ApiError } from "@/lib/api-client";

const {
  fixtures,
  mockCopyToClipboard,
  mockClearRoleScope,
  mockDeleteOrg,
  mockNavigate,
  mockRemoveMember,
  mockSetRoleScope,
  mockToastError,
  mockToastSuccess,
  mockUpdateMember,
  mockUpdateOrg,
  routerState,
} = vi.hoisted(() => {
  const listeners = new Set<() => void>();
  const state: { search: Record<string, unknown> } = { search: {} };
  const routerState = {
    get search() {
      return state.search;
    },
    set: (next: Record<string, unknown>) => {
      state.search = next;
      listeners.forEach((l) => {
        l();
      });
    },
    subscribe: (l: () => void) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    reset: () => {
      state.search = {};
      listeners.forEach((l) => {
        l();
      });
    },
  };
  return {
    fixtures: {
      orgRole: "admin" as "admin" | "member" | "viewer",
      org: {
        id: "org-1",
        display_name: "Testing Org",
        slug: "testing-org",
        avatar_url: null,
        contact_email: null,
        created_at: "2026-04-20T00:00:00Z",
        your_role: "admin" as const,
        member_count: 1,
        remote_credential_integrity_verification_opt_out: false,
      },
      members: [] as MemberResponse[],
      keys: [] as KeyInfo[],
      roleScopes: [] as OrgRoleScope[],
      invites: [
        {
          id: "invite-pending",
          nonce: "ORGINV-PENDING-123",
          role: "member" as const,
          allowed_service_ids: null,
          created_by: "user-1",
          expires_at: "2099-04-25T00:00:00Z",
          redeemed_by: null,
          redeemed_by_email: null,
          redeemed_by_display_name: null,
          redeemed_at: null,
          created_at: "2026-04-20T00:00:00Z",
        },
        {
          id: "invite-redeemed",
          nonce: "ORGINV-REDEEMED-456",
          role: "viewer" as const,
          allowed_service_ids: null,
          created_by: "user-1",
          expires_at: "2099-04-26T00:00:00Z",
          redeemed_by: "user-2",
          redeemed_by_email: "redeemed@example.com",
          redeemed_by_display_name: "Redeemed User",
          redeemed_at: "2026-04-21T00:00:00Z",
          created_at: "2026-04-20T00:00:00Z",
        },
        {
          id: "invite-expired",
          nonce: "ORGINV-EXPIRED-789",
          role: "member" as const,
          allowed_service_ids: null,
          created_by: "user-1",
          expires_at: "2000-01-01T00:00:00Z",
          redeemed_by: null,
          redeemed_by_email: null,
          redeemed_by_display_name: null,
          redeemed_at: null,
          created_at: "2026-04-20T00:00:00Z",
        },
      ],
    },
    mockCopyToClipboard: vi.fn(),
    mockClearRoleScope: vi.fn(),
    mockDeleteOrg: vi.fn(),
    mockNavigate: vi.fn(
      (opts: { search?: Record<string, unknown> } | undefined) => {
        if (opts && typeof opts === "object" && opts.search) {
          routerState.set(opts.search);
        }
      },
    ),
    mockRemoveMember: vi.fn(),
    mockSetRoleScope: vi.fn(),
    mockToastSuccess: vi.fn(),
    mockToastError: vi.fn(),
    mockUpdateMember: vi.fn(),
    mockUpdateOrg: vi.fn(),
    routerState,
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
  useParams: () => ({ orgId: fixtures.org.id }),
  useRouterState: ({
    select,
  }: {
    readonly select: (s: {
      location: { search: Record<string, unknown> };
    }) => unknown;
  }) => {
    return useSyncExternalStore(
      routerState.subscribe,
      () => select({ location: { search: routerState.search } }),
      () => select({ location: { search: {} } }),
    );
  },
}));

vi.mock("@/hooks/use-orgs", () => ({
  useOrg: () => ({
    data: { ...fixtures.org, your_role: fixtures.orgRole },
    isLoading: false,
    error: null,
  }),
  useUpdateOrg: () => ({
    mutateAsync: mockUpdateOrg,
    isPending: false,
  }),
  useDeleteOrg: () => ({
    mutateAsync: mockDeleteOrg,
    isPending: false,
  }),
}));

vi.mock("@/hooks/use-org-members", () => ({
  useOrgMembers: () => ({
    data: fixtures.members,
    isLoading: false,
  }),
  useUpdateMember: () => ({
    mutateAsync: mockUpdateMember,
    isPending: false,
  }),
  useRemoveMember: () => ({
    mutateAsync: mockRemoveMember,
    isPending: false,
  }),
}));

vi.mock("@/hooks/use-org-invites", () => ({
  useOrgInvites: () => ({
    data: fixtures.invites,
    isLoading: false,
  }),
  useCancelInvite: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({
    data: fixtures.keys,
    isLoading: false,
  }),
}));

vi.mock("@/hooks/use-org-role-scopes", () => ({
  useOrgRoleScopes: () => ({
    data: fixtures.roleScopes,
    isLoading: false,
  }),
  useSetOrgRoleScope: () => ({
    mutateAsync: mockSetRoleScope,
    isPending: false,
    variables: undefined,
  }),
  useClearOrgRoleScope: () => ({
    mutateAsync: mockClearRoleScope,
    isPending: false,
    variables: undefined,
  }),
}));

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (
    selector: (state: { user: { id: string } | null }) => unknown,
  ) => selector({ user: { id: "user-1" } }),
}));

vi.mock("@/components/orgs/invite-dialog", () => ({
  InviteDialog: () => null,
}));

vi.mock("@/components/orgs/member-scope-dialog", () => ({
  MemberScopeDialog: () => null,
}));

vi.mock("@/components/orgs/org-approval-configs", () => ({
  OrgApprovalConfigs: () => null,
}));

vi.mock("@/components/orgs/org-avatar", () => ({
  OrgAvatar: () => <div aria-hidden="true" />,
}));

vi.mock("sonner", () => ({
  toast: {
    success: mockToastSuccess,
    error: mockToastError,
  },
}));

vi.mock("@/lib/utils", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/utils")>("@/lib/utils");
  return {
    ...actual,
    copyToClipboard: mockCopyToClipboard,
  };
});

import { OrgDetailPage } from "./org-detail";

const LAST_ADMIN_TOOLTIP =
  "Cannot remove the last active admin. Promote another member to admin first, or delete the organization.";

function renderOrgDetailPage() {
  // The app router mounts this provider globally. These tests render the route
  // page directly, so mirror that root setup here. Zero delay keeps focus-driven
  // tooltip assertions deterministic without changing production defaults.
  return render(
    <TooltipProvider delayDuration={0}>
      <OrgDetailPage />
    </TooltipProvider>,
  );
}

function makeMember(
  userId: string,
  role: OrgRole,
  displayName: string,
): MemberResponse {
  return {
    membership_id: `membership-${userId}`,
    user_id: userId,
    display_name: displayName,
    email: `${userId}@example.com`,
    role,
    scope_source: "inherit",
    allowed_service_ids: null,
    effective_allowed_service_ids: null,
    created_at: "2026-04-20T00:00:00Z",
    revoked_at: null,
  };
}

function makeOrgKey(id: string, label: string, slug: string): KeyInfo {
  return {
    id,
    label,
    slug,
    endpoint_url: `https://${slug}.example.com`,
    endpoint_id: `endpoint-${id}`,
    api_key_id: `api-key-${id}`,
    credential_type: "api_key",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    status: "connected",
    catalog_service_id: null,
    catalog_service_slug: null,
    catalog_service_name: null,
    node_id: null,
    node_priority: 0,
    is_active: true,
    custom_user_agent: null,
    default_request_headers: null,
    ws_frame_injections: [],
    auto_connected: false,
    source_app_id: null,
    source_app_name: null,
    expires_at: null,
    last_used_at: null,
    error_message: null,
    created_at: "2026-04-20T00:00:00Z",
    service_type: "http",
    ssh_host: null,
    ssh_port: null,
    ssh_ca_public_key: null,
    ssh_auth_mode: "proxy_only",
    ssh_allowed_principals: null,
    ssh_certificate_ttl_minutes: null,
    openapi_spec_url: null,
    credential_source: {
      type: "org",
      org_id: fixtures.org.id,
      org_name: fixtures.org.display_name ?? "Testing Org",
      avatar_url: fixtures.org.avatar_url,
      role: "admin",
      allowed: true,
    },
    permission_setup_url: null,
    permission_setup_scopes: null,
  };
}

function makeRoleScope(
  role: OrgRole,
  allowedServiceIds: readonly string[] | null,
): OrgRoleScope {
  return {
    role,
    allowed_service_ids:
      allowedServiceIds === null ? null : [...allowedServiceIds],
    is_default: false,
    updated_at: "2026-04-20T00:00:00Z",
    updated_by: "user-1",
  };
}

function makeApiError(message: string): ApiError {
  return new ApiError(400, {
    error: "bad_request",
    error_code: 4000,
    message,
  });
}

function getRolePermissionCard(roleName: string): HTMLElement {
  const fullAccessToggle = screen.getByRole("switch", {
    name: `Toggle full access for ${roleName}`,
  });
  const card = fullAccessToggle.closest("div.overflow-hidden");
  expect(card).not.toBeNull();
  return card as HTMLElement;
}

function getDesktopMemberRow(displayName: string): HTMLElement {
  const row = screen
    .getAllByText(displayName)
    .map((element) => element.closest("tr"))
    .find((rowElement): rowElement is HTMLElement => rowElement !== null);
  expect(row).toBeDefined();
  return row;
}

async function expectLastAdminTooltip(control: HTMLElement) {
  const wrapper = control.closest('span[tabindex="0"]');
  expect(wrapper).not.toBeNull();
  expect(control).not.toHaveAttribute("title");
  act(() => {
    (wrapper as HTMLElement).focus();
  });
  expect(
    await screen.findByRole("tooltip", {
      name: LAST_ADMIN_TOOLTIP,
    }),
  ).toBeInTheDocument();
  act(() => {
    (wrapper as HTMLElement).blur();
  });
}

function expectNoTooltipWrapper(control: HTMLElement) {
  expect(control.closest('span[tabindex="0"]')).toBeNull();
}

describe("OrgDetailPage", () => {
  const pendingInvite = fixtures.invites[0]!;
  const redeemedInvite = fixtures.invites[1]!;
  const expiredInvite = fixtures.invites[2]!;

  beforeEach(() => {
    vi.clearAllMocks();
    fixtures.orgRole = "admin";
    fixtures.members = [];
    fixtures.keys = [
      makeOrgKey("service-1", "Ops API", "ops-api"),
      makeOrgKey("service-2", "Build API", "build-api"),
      {
        ...makeOrgKey("personal-service", "Personal API", "personal-api"),
        credential_source: { type: "personal" },
      },
    ];
    fixtures.roleScopes = [];
    mockCopyToClipboard.mockResolvedValue(undefined);
    mockClearRoleScope.mockResolvedValue(undefined);
    mockDeleteOrg.mockResolvedValue(undefined);
    mockRemoveMember.mockResolvedValue(undefined);
    mockSetRoleScope.mockResolvedValue({});
    mockUpdateOrg.mockResolvedValue({});
    mockUpdateMember.mockResolvedValue({});
    routerState.reset();
  });

  it("shows org-owned resource tabs to org admins", () => {
    renderOrgDetailPage();

    expect(
      screen.getByRole("tab", { name: "Service Accounts" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: "Developer Apps" }),
    ).toBeInTheDocument();
  });

  it("hides org-owned resource tabs from non-admin members", () => {
    fixtures.orgRole = "member";

    renderOrgDetailPage();

    expect(
      screen.queryByRole("tab", { name: "Service Accounts" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("tab", { name: "Developer Apps" }),
    ).not.toBeInTheDocument();
  });

  it("sends the remote credential integrity opt-out setting only when changed", async () => {
    const user = userEvent.setup();

    renderOrgDetailPage();

    await user.click(screen.getByRole("tab", { name: "Settings" }));
    await user.clear(screen.getByLabelText("Display name"));
    await user.type(
      screen.getByLabelText("Display name"),
      "Testing Org Updated",
    );
    await user.click(screen.getByRole("button", { name: "Save Changes" }));

    await waitFor(() => {
      expect(mockUpdateOrg).toHaveBeenCalledTimes(1);
    });
    expect(mockUpdateOrg.mock.calls[0]?.[0]).toMatchObject({
      orgId: fixtures.org.id,
      body: {
        display_name: "Testing Org Updated",
      },
    });
    expect(mockUpdateOrg.mock.calls[0]?.[0].body).not.toHaveProperty(
      "remote_credential_integrity_verification_opt_out",
    );
    expect(mockUpdateOrg.mock.calls[0]?.[0].body).not.toHaveProperty(
      "contact_email",
    );

    await user.click(
      screen.getByRole("checkbox", {
        name: /opt out of credential accept fingerprint verification/i,
      }),
    );
    await user.click(screen.getByRole("button", { name: "Save Changes" }));

    await waitFor(() => {
      expect(mockUpdateOrg).toHaveBeenCalledTimes(2);
    });
    expect(mockUpdateOrg.mock.calls[1]?.[0].body).toMatchObject({
      remote_credential_integrity_verification_opt_out: true,
    });
  });

  it("sends contact email only when the admin changes it", async () => {
    const user = userEvent.setup();

    renderOrgDetailPage();

    await user.click(screen.getByRole("tab", { name: "Settings" }));
    await user.type(screen.getByLabelText("Contact email"), "ops@example.com");
    await user.click(screen.getByRole("button", { name: "Save Changes" }));

    await waitFor(() => {
      expect(mockUpdateOrg).toHaveBeenCalledTimes(1);
    });
    expect(mockUpdateOrg.mock.calls[0]?.[0]).toMatchObject({
      orgId: fixtures.org.id,
      body: {
        contact_email: "ops@example.com",
      },
    });
  });

  it("shows API validation errors on the settings form", async () => {
    const user = userEvent.setup();
    mockUpdateOrg.mockRejectedValueOnce(makeApiError("Slug is already taken"));

    renderOrgDetailPage();

    await user.click(screen.getByRole("tab", { name: "Settings" }));
    await user.clear(screen.getByLabelText("Slug"));
    await user.type(screen.getByLabelText("Slug"), "taken-slug");
    await user.click(screen.getByRole("button", { name: "Save Changes" }));

    expect(
      await screen.findByText("Slug is already taken"),
    ).toBeInTheDocument();
    expect(mockToastError).not.toHaveBeenCalled();
  });

  it("confirms organization deletion before navigating away", async () => {
    const user = userEvent.setup();

    renderOrgDetailPage();

    await user.click(screen.getByRole("tab", { name: "Settings" }));
    await user.click(
      screen.getByRole("button", { name: "Delete Organization" }),
    );

    const dialog = screen.getByRole("dialog", {
      name: "Delete Organization",
    });
    await user.click(
      within(dialog).getByRole("button", { name: "Delete Organization" }),
    );

    await waitFor(() => {
      expect(mockDeleteOrg).toHaveBeenCalledWith(fixtures.org.id);
    });
    expect(mockToastSuccess).toHaveBeenCalledWith("Organization deleted");
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/orgs" });
  });

  it("copies the full invite join URL for pending invites", async () => {
    const user = userEvent.setup();

    renderOrgDetailPage();

    await user.click(screen.getByRole("tab", { name: "Invites" }));
    // Both mobile and desktop views render a copy button for the pending
    // invite, so pick the first visible one.
    const copyButtons = screen.getAllByRole("button", {
      name: /copy invite link/i,
    });
    await user.click(copyButtons[0]!);

    await waitFor(() => {
      expect(mockCopyToClipboard).toHaveBeenCalledWith(
        `${window.location.origin}/orgs/join/${pendingInvite.nonce}`,
      );
    });
    expect(mockToastSuccess).toHaveBeenCalledWith("Invite link copied");
  });

  it("shows the copy action only for pending invites", async () => {
    const user = userEvent.setup();

    renderOrgDetailPage();

    await user.click(screen.getByRole("tab", { name: "Invites" }));

    // Both mobile and desktop views render a copy button for the single
    // pending invite, so expect 2 (one per responsive breakpoint).
    expect(
      screen.getAllByRole("button", { name: /copy invite link/i }),
    ).toHaveLength(2);
    expect(
      screen.queryByRole("button", {
        name: new RegExp(redeemedInvite.nonce, "i"),
      }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", {
        name: new RegExp(expiredInvite.nonce, "i"),
      }),
    ).not.toBeInTheDocument();
  });

  it("disables remove and role controls for a single active admin", async () => {
    fixtures.members = [makeMember("admin-1", "admin", "Solo Admin")];

    renderOrgDetailPage();

    const removeButton = screen.getByRole("button", {
      name: "Remove Solo Admin",
    });
    expect(removeButton).toBeDisabled();
    await expectLastAdminTooltip(removeButton);

    const roleSelect = screen.getByRole("combobox");
    expect(roleSelect).toBeDisabled();
    await expectLastAdminTooltip(roleSelect);
  });

  it("keeps remove and role controls enabled when there are two active admins", () => {
    fixtures.members = [
      makeMember("admin-1", "admin", "First Admin"),
      makeMember("admin-2", "admin", "Second Admin"),
    ];

    renderOrgDetailPage();

    const firstRemove = screen.getByRole("button", {
      name: "Remove First Admin",
    });
    const secondRemove = screen.getByRole("button", {
      name: "Remove Second Admin",
    });
    expect(firstRemove).toBeEnabled();
    expect(secondRemove).toBeEnabled();
    expectNoTooltipWrapper(firstRemove);
    expectNoTooltipWrapper(secondRemove);
    for (const roleSelect of screen.getAllByRole("combobox")) {
      expect(roleSelect).toBeEnabled();
      expectNoTooltipWrapper(roleSelect);
    }
  });

  it("locks only the admin row when a single active admin has non-admin peers", async () => {
    fixtures.members = [
      makeMember("admin-1", "admin", "Solo Admin"),
      makeMember("member-1", "member", "Member User"),
    ];

    renderOrgDetailPage();

    const adminRemove = screen.getByRole("button", {
      name: "Remove Solo Admin",
    });
    const memberRemove = screen.getByRole("button", {
      name: "Remove Member User",
    });
    expect(adminRemove).toBeDisabled();
    expect(memberRemove).toBeEnabled();
    await expectLastAdminTooltip(adminRemove);
    expectNoTooltipWrapper(memberRemove);

    const roleSelects = screen.getAllByRole("combobox");
    expect(roleSelects[0]).toBeDisabled();
    expect(roleSelects[1]).toBeEnabled();
    await expectLastAdminTooltip(roleSelects[0]!);
    expectNoTooltipWrapper(roleSelects[1]!);
  });

  it("updates a member role through the member row control", async () => {
    const user = userEvent.setup();
    fixtures.members = [
      makeMember("admin-1", "admin", "Org Admin"),
      makeMember("member-1", "member", "Member User"),
    ];

    renderOrgDetailPage();

    const memberRow = getDesktopMemberRow("Member User");
    await user.click(within(memberRow).getByRole("combobox"));
    await user.click(await screen.findByRole("option", { name: "Viewer" }));

    await waitFor(() => {
      expect(mockUpdateMember).toHaveBeenCalledTimes(1);
    });
    expect(mockUpdateMember).toHaveBeenCalledWith({
      orgId: fixtures.org.id,
      memberId: "member-1",
      body: { role: "viewer" },
    });
    expect(mockToastSuccess).toHaveBeenCalledWith("Role updated");
  });

  it("surfaces API errors from member role updates", async () => {
    const user = userEvent.setup();
    fixtures.members = [
      makeMember("admin-1", "admin", "Org Admin"),
      makeMember("member-1", "member", "Member User"),
    ];
    mockUpdateMember.mockRejectedValueOnce(
      makeApiError("Cannot demote this member"),
    );

    renderOrgDetailPage();

    const memberRow = getDesktopMemberRow("Member User");
    await user.click(within(memberRow).getByRole("combobox"));
    await user.click(await screen.findByRole("option", { name: "Viewer" }));

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith("Cannot demote this member");
    });
  });

  it("confirms member removal before calling the remove mutation", async () => {
    const user = userEvent.setup();
    fixtures.members = [
      makeMember("admin-1", "admin", "Org Admin"),
      makeMember("member-1", "member", "Member User"),
    ];

    renderOrgDetailPage();

    await user.click(
      screen.getByRole("button", { name: "Remove Member User" }),
    );
    const dialog = screen.getByRole("dialog", { name: "Remove member" });
    await user.click(within(dialog).getByRole("button", { name: "Remove" }));

    await waitFor(() => {
      expect(mockRemoveMember).toHaveBeenCalledWith({
        orgId: fixtures.org.id,
        memberId: "member-1",
      });
    });
    expect(mockToastSuccess).toHaveBeenCalledWith("Member removed");
  });

  it("resets a custom member service scope to inherited role defaults", async () => {
    const user = userEvent.setup();
    fixtures.members = [
      makeMember("admin-1", "admin", "Org Admin"),
      {
        ...makeMember("member-1", "member", "Member User"),
        scope_source: "override",
        allowed_service_ids: ["service-1"],
        effective_allowed_service_ids: ["service-1"],
      },
    ];

    renderOrgDetailPage();

    await user.click(
      screen.getByRole("button", {
        name: "Reset Member User to role defaults",
      }),
    );

    await waitFor(() => {
      expect(mockUpdateMember).toHaveBeenCalledWith({
        orgId: fixtures.org.id,
        memberId: "member-1",
        body: { scope_source: "inherit" },
      });
    });
    await waitFor(() => {
      expect(mockToastSuccess).toHaveBeenCalledWith(
        "Member reset to role defaults",
      );
    });
  });

  it("limits role permission management to org admins", async () => {
    const user = userEvent.setup();
    fixtures.orgRole = "member";

    renderOrgDetailPage();

    await user.click(screen.getByRole("tab", { name: "Role permissions" }));

    expect(
      screen.getByText("Only admins can manage role permissions."),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("switch", { name: /Toggle full access for Member/i }),
    ).not.toBeInTheDocument();
  });

  it("saves restricted service access through the role-scope set mutation", async () => {
    const user = userEvent.setup();
    fixtures.roleScopes = [makeRoleScope("member", ["service-1"])];

    renderOrgDetailPage();

    await user.click(screen.getByRole("tab", { name: "Role permissions" }));
    const memberCard = getRolePermissionCard("Member");
    await user.click(
      within(memberCard).getByRole("checkbox", { name: /Build API/i }),
    );
    await user.click(within(memberCard).getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(mockSetRoleScope).toHaveBeenCalledWith({
        role: "member",
        body: { allowed_service_ids: ["service-1", "service-2"] },
      });
    });
    expect(mockClearRoleScope).not.toHaveBeenCalled();
    expect(mockToastSuccess).toHaveBeenCalledWith("Role permissions updated");
  });

  it("saves full role access through the role-scope clear mutation", async () => {
    const user = userEvent.setup();
    fixtures.roleScopes = [makeRoleScope("member", ["service-1"])];

    renderOrgDetailPage();

    await user.click(screen.getByRole("tab", { name: "Role permissions" }));
    const memberCard = getRolePermissionCard("Member");
    await user.click(
      within(memberCard).getByRole("switch", {
        name: "Toggle full access for Member",
      }),
    );
    await user.click(within(memberCard).getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(mockClearRoleScope).toHaveBeenCalledWith({ role: "member" });
    });
    expect(mockSetRoleScope).not.toHaveBeenCalled();
    expect(mockToastSuccess).toHaveBeenCalledWith("Role permissions updated");
  });

  it("resets unsaved role permission drafts back to persisted scope", async () => {
    const user = userEvent.setup();
    fixtures.roleScopes = [makeRoleScope("member", ["service-1"])];

    renderOrgDetailPage();

    await user.click(screen.getByRole("tab", { name: "Role permissions" }));
    const memberCard = getRolePermissionCard("Member");
    const opsService = within(memberCard).getByRole("checkbox", {
      name: /Ops API/i,
    });
    const buildService = within(memberCard).getByRole("checkbox", {
      name: /Build API/i,
    });

    await user.click(buildService);
    expect(buildService).toHaveAttribute("aria-checked", "true");
    expect(
      within(memberCard).getByRole("button", { name: "Save" }),
    ).toBeEnabled();

    await user.click(within(memberCard).getByRole("button", { name: "Reset" }));

    expect(opsService).toHaveAttribute("aria-checked", "true");
    expect(buildService).toHaveAttribute("aria-checked", "false");
    expect(
      within(memberCard).getByRole("button", { name: "Save" }),
    ).toBeDisabled();
    expect(mockSetRoleScope).not.toHaveBeenCalled();
    expect(mockClearRoleScope).not.toHaveBeenCalled();
  });

  it("surfaces API errors from role permission saves", async () => {
    const user = userEvent.setup();
    fixtures.roleScopes = [makeRoleScope("member", ["service-1"])];
    mockSetRoleScope.mockRejectedValueOnce(
      makeApiError("Role scope update denied"),
    );

    renderOrgDetailPage();

    await user.click(screen.getByRole("tab", { name: "Role permissions" }));
    const memberCard = getRolePermissionCard("Member");
    await user.click(
      within(memberCard).getByRole("checkbox", { name: /Build API/i }),
    );
    await user.click(within(memberCard).getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith("Role scope update denied");
    });
    expect(mockToastSuccess).not.toHaveBeenCalledWith(
      "Role permissions updated",
    );
  });
});

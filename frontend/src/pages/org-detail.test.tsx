import type { ReactNode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSyncExternalStore } from "react";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { MemberResponse, OrgRole } from "@/schemas/orgs";

const {
  fixtures,
  mockCopyToClipboard,
  mockNavigate,
  mockToastError,
  mockToastSuccess,
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
      avatar_url: null,
      contact_email: null,
      created_at: "2026-04-20T00:00:00Z",
      your_role: "admin" as const,
      member_count: 1,
    },
    members: [] as MemberResponse[],
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
  mockNavigate: vi.fn(
    (opts: { search?: Record<string, unknown> } | undefined) => {
      if (opts && typeof opts === "object" && opts.search) {
        routerState.set(opts.search);
      }
    },
  ),
  mockToastSuccess: vi.fn(),
  mockToastError: vi.fn(),
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
    readonly select: (s: { location: { search: Record<string, unknown> } }) => unknown;
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
    mutateAsync: vi.fn(),
    isPending: false,
  }),
  useDeleteOrg: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

vi.mock("@/hooks/use-org-members", () => ({
  useOrgMembers: () => ({
    data: fixtures.members,
    isLoading: false,
  }),
  useUpdateMember: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
  useRemoveMember: () => ({
    mutateAsync: vi.fn(),
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
    mockCopyToClipboard.mockResolvedValue(undefined);
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

  it("copies the full invite join URL for pending invites", async () => {
    const user = userEvent.setup();

    renderOrgDetailPage();

    await user.click(screen.getByRole("tab", { name: "Invites" }));
    // Both mobile and desktop views render a copy button for the pending
    // invite, so pick the first visible one.
    const copyButtons = screen.getAllByRole("button", { name: /copy invite link/i });
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
});

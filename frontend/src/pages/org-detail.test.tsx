import type { ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  fixtures,
  mockCopyToClipboard,
  mockNavigate,
  mockToastError,
  mockToastSuccess,
} = vi.hoisted(() => ({
  fixtures: {
    org: {
      id: "org-1",
      display_name: "Testing Org",
      avatar_url: null,
      contact_email: null,
      created_at: "2026-04-20T00:00:00Z",
      your_role: "admin" as const,
      member_count: 1,
    },
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
  mockNavigate: vi.fn(),
  mockToastSuccess: vi.fn(),
  mockToastError: vi.fn(),
}));

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
}));

vi.mock("@/hooks/use-orgs", () => ({
  useOrg: () => ({
    data: fixtures.org,
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
    data: [],
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
  const actual = await vi.importActual<typeof import("@/lib/utils")>(
    "@/lib/utils",
  );
  return {
    ...actual,
    copyToClipboard: mockCopyToClipboard,
  };
});

import { OrgDetailPage } from "./org-detail";

describe("OrgDetailPage invites tab", () => {
  const pendingInvite = fixtures.invites[0]!;
  const redeemedInvite = fixtures.invites[1]!;
  const expiredInvite = fixtures.invites[2]!;

  beforeEach(() => {
    vi.clearAllMocks();
    mockCopyToClipboard.mockResolvedValue(undefined);
  });

  it("copies the full invite join URL for pending invites", async () => {
    const user = userEvent.setup();

    render(<OrgDetailPage />);

    await user.click(screen.getByRole("tab", { name: "Invites" }));
    await user.click(screen.getByRole("button", { name: /copy invite link/i }));

    await waitFor(() => {
      expect(mockCopyToClipboard).toHaveBeenCalledWith(
        `${window.location.origin}/orgs/join/${pendingInvite.nonce}`,
      );
    });
    expect(mockToastSuccess).toHaveBeenCalledWith("Invite link copied");
  });

  it("shows the copy action only for pending invites", async () => {
    const user = userEvent.setup();

    render(<OrgDetailPage />);

    await user.click(screen.getByRole("tab", { name: "Invites" }));

    expect(
      screen.getAllByRole("button", { name: /copy invite link/i }),
    ).toHaveLength(1);
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
});

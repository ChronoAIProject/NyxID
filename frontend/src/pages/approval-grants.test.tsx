import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type {
  ApprovalGrantItem,
  DominantOrgPolicy,
  ServiceApprovalConfigItem,
} from "@/types/approvals";

const {
  fixtures,
  mockUseApprovalGrants,
  mockUseServiceApprovalConfigs,
  mockRevokeMutateAsync,
  mockRefetch,
  mockToastError,
  mockToastSuccess,
} = vi.hoisted(() => ({
  fixtures: {
    grants: [] as ApprovalGrantItem[],
    total: 0,
    isLoading: false,
    error: null as unknown,
    configs: [] as ServiceApprovalConfigItem[],
    dominantOrgPolicies: [] as DominantOrgPolicy[],
    configsLoading: false,
    configsError: null as unknown,
    revokeRejection: null as Error | null,
  },
  mockUseApprovalGrants: vi.fn(),
  mockUseServiceApprovalConfigs: vi.fn(),
  mockRevokeMutateAsync: vi.fn(),
  mockRefetch: vi.fn(),
  mockToastError: vi.fn(),
  mockToastSuccess: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
  }: {
    readonly children: ReactNode;
    readonly to: string;
  }) => (
    <a href={to} data-router-link="true">
      {children}
    </a>
  ),
}));

vi.mock("@/hooks/use-approvals", () => ({
  useApprovalGrants: (
    page: number,
    perPage: number,
    options?: { includeAdminOrgs?: boolean; orgId?: string },
  ) => mockUseApprovalGrants(page, perPage, options),
  useServiceApprovalConfigs: () => mockUseServiceApprovalConfigs(),
  useRevokeGrant: () => ({
    mutateAsync: mockRevokeMutateAsync,
    isPending: false,
  }),
}));

// ApiError must be a real class so `err instanceof ApiError` in the page works.
// Mirror the real (status, response) constructor signature so the page reads
// `err.message` exactly as it would in production.
vi.mock("@/lib/api-client", () => ({
  ApiError: class ApiError extends Error {
    readonly status: number;
    constructor(status: number, response: { message: string }) {
      super(response.message);
      this.name = "ApiError";
      this.status = status;
    }
  },
}));

vi.mock("sonner", () => ({
  toast: {
    success: mockToastSuccess,
    error: mockToastError,
  },
}));

import { ApiError } from "@/lib/api-client";
import { ApprovalGrantsPage } from "./approval-grants";

function makeGrant(
  overrides: Partial<ApprovalGrantItem> = {},
): ApprovalGrantItem {
  return {
    id: "grant-1",
    service_id: "svc-1",
    service_name: "OpenAI",
    requester_type: "api_key",
    requester_id: "key-1",
    requester_label: "coding-agent",
    org_scoped: false,
    granted_at: "2026-05-01T00:00:00Z",
    // Far future so isExpiringSoon() is false unless a test overrides it.
    expires_at: "2099-01-01T00:00:00Z",
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  fixtures.grants = [];
  fixtures.total = 0;
  fixtures.isLoading = false;
  fixtures.error = null;
  fixtures.configs = [];
  fixtures.dominantOrgPolicies = [];
  fixtures.configsLoading = false;
  fixtures.configsError = null;
  fixtures.revokeRejection = null;
  mockRevokeMutateAsync.mockImplementation(() =>
    fixtures.revokeRejection
      ? Promise.reject(fixtures.revokeRejection)
      : Promise.resolve({}),
  );
  mockUseApprovalGrants.mockImplementation(() => ({
    data: {
      grants: fixtures.grants,
      total: fixtures.total,
      page: 1,
      per_page: 20,
    },
    isLoading: fixtures.isLoading,
    error: fixtures.error,
    refetch: mockRefetch,
  }));
  mockUseServiceApprovalConfigs.mockImplementation(() => ({
    data: {
      configs: fixtures.configs,
      dominant_org_policies: fixtures.dominantOrgPolicies,
    },
    isLoading: fixtures.configsLoading,
    error: fixtures.configsError,
  }));
});

function makeConfig(
  overrides: Partial<ServiceApprovalConfigItem> = {},
): ServiceApprovalConfigItem {
  return {
    service_id: "svc-1",
    service_name: "OpenAI",
    approval_required: true,
    approval_mode: "per_request",
    rules: [],
    default_effect: null,
    created_at: "2026-05-01T00:00:00Z",
    updated_at: "2026-05-01T00:00:00Z",
    ...overrides,
  };
}

describe("ApprovalGrantsPage", () => {
  it("requests the first page with a perPage of 20", () => {
    render(<ApprovalGrantsPage />);
    expect(mockUseApprovalGrants).toHaveBeenCalledWith(1, 20, {
      includeAdminOrgs: true,
    });
  });

  it("renders a row per grant with service name, requester label, and type", () => {
    fixtures.grants = [
      makeGrant({
        id: "g1",
        service_name: "OpenAI",
        requester_label: "coding-agent",
        requester_type: "api_key",
      }),
      makeGrant({
        id: "g2",
        service_name: "GitHub",
        requester_label: null,
        requester_type: "user",
      }),
    ];
    fixtures.total = 2;

    render(<ApprovalGrantsPage />);

    const table = screen.getByRole("table");
    // Desktop table shows one data row per grant.
    expect(within(table).getByText("OpenAI")).toBeInTheDocument();
    expect(within(table).getByText("GitHub")).toBeInTheDocument();
    // requester_label is shown when present; falls back to requester_type otherwise.
    expect(within(table).getByText("coding-agent")).toBeInTheDocument();
    // g2 has null label, so the type "user" stands in for the label and also
    // appears as the secondary type line.
    expect(within(table).getAllByText("user")).toHaveLength(2);
    // g1's type line.
    expect(within(table).getByText("api_key")).toBeInTheDocument();
  });

  it("shows the 'Expiring soon' badge only for grants expiring within 3 days", () => {
    const soon = new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString();
    const later = new Date(Date.now() + 10 * 24 * 60 * 60 * 1000).toISOString();
    fixtures.grants = [
      makeGrant({ id: "soon", service_name: "ExpiringSvc", expires_at: soon }),
      makeGrant({ id: "later", service_name: "SafeSvc", expires_at: later }),
    ];
    fixtures.total = 2;

    render(<ApprovalGrantsPage />);

    // Mobile + desktop both render the badge for the expiring grant -> 2 copies.
    expect(screen.getAllByText("Expiring soon")).toHaveLength(2);
  });

  it("revoke flow confirms then calls useRevokeGrant with the row's grantId", async () => {
    const user = userEvent.setup();
    fixtures.grants = [makeGrant({ id: "grant-42", service_name: "OpenAI" })];
    fixtures.total = 1;

    render(<ApprovalGrantsPage />);

    // Opening the dialog: the desktop revoke button has a Trash icon, no name.
    // Both mobile and desktop render one; clicking either opens the dialog.
    const revokeButtons = screen.getAllByRole("button");
    // The dialog is not open until a revoke trigger fires.
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    // Trigger via the desktop table trash button (icon-only ghost button).
    const trashTriggers = revokeButtons.filter(
      (b) => b.querySelector("svg.lucide-trash2") !== null,
    );
    expect(trashTriggers.length).toBeGreaterThan(0);
    await user.click(trashTriggers[0]!);

    const dialog = await screen.findByRole("dialog");
    // Confirmation copy names the targeted service.
    expect(
      within(dialog).getByText(/Revoke the grant for "OpenAI"/i),
    ).toBeInTheDocument();

    await user.click(
      within(dialog).getByRole("button", { name: "Revoke Grant" }),
    );

    await waitFor(() => {
      expect(mockRevokeMutateAsync).toHaveBeenCalledWith({
        grantId: "grant-42",
      });
    });
    expect(mockToastSuccess).toHaveBeenCalledWith("Grant revoked");
  });

  it("renders an org badge in both layouts and revokes with the owning org id", async () => {
    const user = userEvent.setup();
    fixtures.grants = [
      makeGrant({
        id: "org-grant",
        service_name: "GitHub",
        org_scoped: true,
        org_id: "org-42",
        org_name: "Acme",
      }),
    ];
    fixtures.total = 1;

    render(<ApprovalGrantsPage />);

    expect(screen.getAllByText("Org: Acme")).toHaveLength(2);
    const revokeButtons = screen.getAllByRole("button", {
      name: "Revoke grant for GitHub",
    });
    expect(revokeButtons).toHaveLength(2);
    await user.click(revokeButtons[0]!);
    const dialog = await screen.findByRole("dialog");
    await user.click(
      within(dialog).getByRole("button", { name: "Revoke Grant" }),
    );

    await waitFor(() => {
      expect(mockRevokeMutateAsync).toHaveBeenCalledWith({
        grantId: "org-grant",
        orgId: "org-42",
      });
    });
  });

  it("surfaces the ApiError message via toast.error when revoke fails", async () => {
    const user = userEvent.setup();
    fixtures.grants = [makeGrant({ id: "grant-99", service_name: "OpenAI" })];
    fixtures.total = 1;
    fixtures.revokeRejection = new ApiError(409, {
      error: "conflict",
      error_code: 4090,
      message: "Grant already revoked",
    });

    render(<ApprovalGrantsPage />);

    const trashTriggers = screen
      .getAllByRole("button")
      .filter((b) => b.querySelector("svg.lucide-trash2") !== null);
    await user.click(trashTriggers[0]!);
    const dialog = await screen.findByRole("dialog");
    await user.click(
      within(dialog).getByRole("button", { name: "Revoke Grant" }),
    );

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith("Grant already revoked");
    });
    expect(mockToastSuccess).not.toHaveBeenCalled();
  });

  it("links to approval settings when no service supports grant mode", () => {
    fixtures.grants = [];
    fixtures.total = 0;

    render(<ApprovalGrantsPage />);

    expect(screen.getByText("Grant Mode Is Off")).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Configure approval policies" }),
    ).toHaveAttribute("data-router-link", "true");
    expect(
      screen.getByRole("link", { name: "Configure approval policies" }),
    ).toHaveAttribute("href", "/approvals/settings");
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("distinguishes grant-enabled services with no active grants", () => {
    fixtures.grants = [];
    fixtures.total = 0;
    fixtures.configs = [makeConfig({ approval_mode: "grant" })];

    render(<ApprovalGrantsPage />);

    expect(screen.getByText("No Active Grants")).toBeInTheDocument();
    expect(
      screen.getByText(/grant-enabled services have no active grants/i),
    ).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("explains dominant org policies and links members to policy settings", () => {
    fixtures.dominantOrgPolicies = [
      { org_id: "org-1", service_id: "svc-1" },
    ];

    render(<ApprovalGrantsPage />);

    expect(screen.getByText("No Active Grants")).toBeInTheDocument();
    expect(screen.getByText(/managed by an org admin/i)).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Configure approval policies" }),
    ).toHaveAttribute("data-router-link", "true");
  });

  it("distinguishes an unavailable approval-policy response", () => {
    fixtures.configsError = new Error("config response failed");

    render(<ApprovalGrantsPage />);

    expect(screen.getByText("Policy Status Unavailable")).toBeInTheDocument();
    expect(
      screen.getByText(/approval policy status could not be loaded/i),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("link", { name: "Configure approval policies" }),
    ).not.toBeInTheDocument();
  });

  it("renders skeletons (no table, no empty state) while loading", () => {
    fixtures.isLoading = true;

    render(<ApprovalGrantsPage />);

    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(screen.queryByText("No Active Grants")).not.toBeInTheDocument();
    // Loading branch never reads grants/error.
    expect(
      screen.queryByText("Failed to load grants. Please try again."),
    ).not.toBeInTheDocument();
  });

  it("renders an error banner with a retry that calls refetch", async () => {
    const user = userEvent.setup();
    fixtures.error = new Error("boom");

    render(<ApprovalGrantsPage />);

    expect(
      screen.getByText("Failed to load grants. Please try again."),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /retry/i }));
    expect(mockRefetch).toHaveBeenCalled();
  });

  it("hides pagination when there is a single page of results", () => {
    fixtures.grants = [makeGrant()];
    fixtures.total = 1;

    render(<ApprovalGrantsPage />);

    expect(
      screen.queryByRole("button", { name: "Next page" }),
    ).not.toBeInTheDocument();
  });

  it("shows pagination controls when total exceeds one page", () => {
    fixtures.grants = [makeGrant()];
    fixtures.total = 25; // 25 / 20 perPage => 2 pages

    render(<ApprovalGrantsPage />);

    const next = screen.getByRole("button", { name: "Next page" });
    const prev = screen.getByRole("button", { name: "Previous page" });
    expect(next).toBeEnabled();
    // On page 1 the previous control is disabled.
    expect(prev).toBeDisabled();
    expect(screen.getByText(/Page 1 of 2/)).toBeInTheDocument();
  });

  it("advancing a page re-queries useApprovalGrants for page 2", async () => {
    const user = userEvent.setup();
    fixtures.grants = [makeGrant()];
    fixtures.total = 25;

    render(<ApprovalGrantsPage />);

    await user.click(screen.getByRole("button", { name: "Next page" }));

    await waitFor(() => {
      expect(mockUseApprovalGrants).toHaveBeenCalledWith(2, 20, {
        includeAdminOrgs: true,
      });
    });
  });
});

import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { ApprovalRequestItem } from "@/types/approvals";

const {
  fixtures,
  mockUseApprovalRequests,
  mockDecideMutateAsync,
  mockRefetch,
  mockToastError,
  mockToastSuccess,
} = vi.hoisted(() => ({
  fixtures: {
    requests: [] as ApprovalRequestItem[],
    total: 0,
    isLoading: false,
    error: null as unknown,
    decideRejection: null as Error | null,
  },
  mockUseApprovalRequests: vi.fn(),
  mockDecideMutateAsync: vi.fn(),
  mockRefetch: vi.fn(),
  mockToastError: vi.fn(),
  mockToastSuccess: vi.fn(),
}));

vi.mock("@/hooks/use-approvals", () => ({
  useApprovalRequests: (page: number, perPage: number, status?: string) =>
    mockUseApprovalRequests(page, perPage, status),
  useDecideApproval: () => ({
    mutateAsync: mockDecideMutateAsync,
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
import { ApprovalHistoryPage } from "./approval-history";

function makeRequest(
  overrides: Partial<ApprovalRequestItem> = {},
): ApprovalRequestItem {
  return {
    id: "req-1",
    service_name: "OpenAI",
    service_slug: "openai",
    requester_type: "api_key",
    requester_label: "coding-agent",
    operation_summary: "POST /v1/chat/completions",
    action_description: null,
    tool_name: null,
    tool_call_id: null,
    tool_arguments: null,
    is_destructive: null,
    approval_mode: "per_request",
    status: "pending",
    created_at: "2026-05-01T00:00:00Z",
    decided_at: null,
    decision_channel: null,
    ...overrides,
  };
}

// Radix Select relies on pointer-capture + scrollIntoView, neither of which
// happy-dom implements. Polyfill them so the dropdown can actually open.
beforeAll(() => {
  if (!Element.prototype.hasPointerCapture) {
    Element.prototype.hasPointerCapture = () => false;
  }
  if (!Element.prototype.setPointerCapture) {
    Element.prototype.setPointerCapture = () => {};
  }
  if (!Element.prototype.releasePointerCapture) {
    Element.prototype.releasePointerCapture = () => {};
  }
  if (!Element.prototype.scrollIntoView) {
    Element.prototype.scrollIntoView = () => {};
  }
});

beforeEach(() => {
  vi.clearAllMocks();
  fixtures.requests = [];
  fixtures.total = 0;
  fixtures.isLoading = false;
  fixtures.error = null;
  fixtures.decideRejection = null;
  mockDecideMutateAsync.mockImplementation(() =>
    fixtures.decideRejection
      ? Promise.reject(fixtures.decideRejection)
      : Promise.resolve({}),
  );
  mockUseApprovalRequests.mockImplementation(() => ({
    data: {
      requests: fixtures.requests,
      total: fixtures.total,
      page: 1,
      per_page: 20,
    },
    isLoading: fixtures.isLoading,
    error: fixtures.error,
    refetch: mockRefetch,
  }));
});

describe("ApprovalHistoryPage", () => {
  it("requests the first page, perPage 20, and no status filter by default", () => {
    render(<ApprovalHistoryPage />);
    // "all" maps to undefined so the hook is not narrowed by status.
    expect(mockUseApprovalRequests).toHaveBeenCalledWith(1, 20, undefined);
  });

  it("renders one table row per proxy request with service, slug, requester, action, status", () => {
    fixtures.requests = [
      makeRequest({
        id: "r1",
        service_name: "OpenAI",
        service_slug: "openai",
        requester_label: "coding-agent",
        requester_type: "api_key",
        operation_summary: "POST /v1/chat/completions",
        action_description: "Generate a chat completion",
        status: "approved",
        created_at: "2026-05-01T00:00:00Z",
        decided_at: "2026-05-02T00:00:00Z",
      }),
      makeRequest({
        id: "r2",
        service_name: "GitHub",
        service_slug: "github",
        requester_label: null,
        requester_type: "user",
        operation_summary: "GET /repos",
        status: "rejected",
      }),
    ];
    fixtures.total = 2;

    render(<ApprovalHistoryPage />);

    const table = screen.getByRole("table");
    expect(within(table).getByText("OpenAI")).toBeInTheDocument();
    expect(within(table).getByText("openai")).toBeInTheDocument();
    expect(within(table).getByText("GitHub")).toBeInTheDocument();
    // requester_label is shown when present; falls back to requester_type.
    expect(within(table).getByText("coding-agent")).toBeInTheDocument();
    // r2 has a null label, so "user" stands in for the label and is also the
    // secondary type line -> 2 occurrences.
    expect(within(table).getAllByText("user")).toHaveLength(2);
    // action_description takes precedence over operation_summary as the title.
    expect(
      within(table).getByText("Generate a chat completion"),
    ).toBeInTheDocument();
    // ...with operation_summary shown as the secondary line beneath it.
    expect(
      within(table).getByText("POST /v1/chat/completions"),
    ).toBeInTheDocument();
    // r2 has no action_description, so its operation_summary is the title.
    expect(within(table).getByText("GET /repos")).toBeInTheDocument();
    // Distinct status badges per row.
    expect(within(table).getByText("Approved")).toBeInTheDocument();
    expect(within(table).getByText("Rejected")).toBeInTheDocument();
    // Decided cell renders "-" for the row that has no decided_at.
    expect(within(table).getByText("-")).toBeInTheDocument();
  });

  it("renders tool approvals with the tool name and a Destructive badge", () => {
    fixtures.requests = [
      makeRequest({
        id: "tool-1",
        tool_name: "delete_repository",
        tool_arguments: '{"repo":"acme/site"}',
        is_destructive: true,
        status: "approved",
      }),
    ];
    fixtures.total = 1;

    render(<ApprovalHistoryPage />);

    const table = screen.getByRole("table");
    expect(within(table).getByText("delete_repository")).toBeInTheDocument();
    // Tool rows use a fixed action title plus the serialized arguments line.
    expect(within(table).getByText("Tool execution approval")).toBeInTheDocument();
    expect(within(table).getByText('{"repo":"acme/site"}')).toBeInTheDocument();
    // is_destructive surfaces a Destructive badge.
    expect(within(table).getByText("Destructive")).toBeInTheDocument();
  });

  it("changing the status filter re-queries the hook with that status and resets to page 1", async () => {
    const user = userEvent.setup();
    fixtures.requests = [makeRequest()];
    fixtures.total = 1;

    render(<ApprovalHistoryPage />);

    expect(mockUseApprovalRequests).toHaveBeenLastCalledWith(1, 20, undefined);

    await user.click(screen.getByRole("combobox"));
    await user.click(await screen.findByRole("option", { name: "Approved" }));

    await waitFor(() => {
      expect(mockUseApprovalRequests).toHaveBeenLastCalledWith(
        1,
        20,
        "approved",
      );
    });
  });

  it("approving a pending request confirms then calls useDecideApproval with approved=true", async () => {
    const user = userEvent.setup();
    fixtures.requests = [
      makeRequest({ id: "req-42", service_name: "OpenAI", status: "pending" }),
    ];
    fixtures.total = 1;

    render(<ApprovalHistoryPage />);

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    // Desktop table + mobile card each render an Approve button; either opens
    // the same confirmation dialog. Use the first.
    const approveTriggers = screen.getAllByRole("button", { name: /approve/i });
    await user.click(approveTriggers[0]!);

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText(/approve access to "OpenAI"/i),
    ).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "Approve" }));

    await waitFor(() => {
      expect(mockDecideMutateAsync).toHaveBeenCalledWith({
        requestId: "req-42",
        approved: true,
      });
    });
    expect(mockToastSuccess).toHaveBeenCalledWith("Request approved");
  });

  it("shows the reusable-grant copy in the approve dialog for a grant-mode request", async () => {
    const user = userEvent.setup();
    fixtures.requests = [
      makeRequest({
        id: "req-grant",
        service_name: "OpenAI",
        status: "pending",
        approval_mode: "grant",
      }),
    ];
    fixtures.total = 1;

    render(<ApprovalHistoryPage />);

    const approveTriggers = screen.getAllByRole("button", { name: /approve/i });
    await user.click(approveTriggers[0]!);

    const dialog = await screen.findByRole("dialog");
    // grant mode swaps the per-request copy for the reusable-grant copy.
    expect(
      within(dialog).getByText(
        /A reusable approval grant will be created using your configured expiry\./i,
      ),
    ).toBeInTheDocument();
    // ...and the per-request sentence is NOT shown for a grant-mode request.
    expect(
      within(dialog).queryByText(
        /This approval applies only to the current request\./i,
      ),
    ).not.toBeInTheDocument();
  });

  it("surfaces the ApiError message via toast.error when a decision fails", async () => {
    const user = userEvent.setup();
    fixtures.requests = [
      makeRequest({ id: "req-99", service_name: "OpenAI", status: "pending" }),
    ];
    fixtures.total = 1;
    fixtures.decideRejection = new ApiError(409, {
      error: "conflict",
      error_code: 4090,
      message: "Request already decided",
    });

    render(<ApprovalHistoryPage />);

    const rejectTriggers = screen.getAllByRole("button", { name: /reject/i });
    await user.click(rejectTriggers[0]!);
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Reject" }));

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith("Request already decided");
    });
    expect(mockToastSuccess).not.toHaveBeenCalled();
  });

  it("renders the empty state and no table when there are no requests", () => {
    fixtures.requests = [];
    fixtures.total = 0;

    render(<ApprovalHistoryPage />);

    expect(screen.getByText("No Approval Requests")).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("renders skeletons (no table, no empty state) while loading", () => {
    fixtures.isLoading = true;

    render(<ApprovalHistoryPage />);

    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(screen.queryByText("No Approval Requests")).not.toBeInTheDocument();
    expect(
      screen.queryByText("Failed to load approval history. Please try again."),
    ).not.toBeInTheDocument();
  });

  it("renders an error banner with a retry that calls refetch", async () => {
    const user = userEvent.setup();
    fixtures.error = new Error("boom");

    render(<ApprovalHistoryPage />);

    expect(
      screen.getByText("Failed to load approval history. Please try again."),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /retry/i }));
    expect(mockRefetch).toHaveBeenCalled();
  });

  it("hides pagination when there is a single page of results", () => {
    fixtures.requests = [makeRequest()];
    fixtures.total = 1;

    render(<ApprovalHistoryPage />);

    expect(
      screen.queryByRole("button", { name: "Next page" }),
    ).not.toBeInTheDocument();
  });

  it("shows pagination controls (prev disabled on page 1) when total exceeds one page", () => {
    fixtures.requests = [makeRequest()];
    fixtures.total = 25; // 25 / 20 perPage => 2 pages

    render(<ApprovalHistoryPage />);

    const next = screen.getByRole("button", { name: "Next page" });
    const prev = screen.getByRole("button", { name: "Previous page" });
    expect(next).toBeEnabled();
    expect(prev).toBeDisabled();
    expect(screen.getByText(/Page 1 of 2/)).toBeInTheDocument();
  });

  it("advancing a page re-queries useApprovalRequests for page 2", async () => {
    const user = userEvent.setup();
    fixtures.requests = [makeRequest()];
    fixtures.total = 25;

    render(<ApprovalHistoryPage />);

    await user.click(screen.getByRole("button", { name: "Next page" }));

    await waitFor(() => {
      expect(mockUseApprovalRequests).toHaveBeenLastCalledWith(2, 20, undefined);
    });
  });
});

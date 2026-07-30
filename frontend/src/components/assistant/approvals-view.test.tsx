import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { ApprovalRequestItem } from "@/types/approvals";
import { ApprovalsView } from "./approvals-view";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  ApiError: class MockApiError extends Error {},
  api: { get: mockGet, post: mockPost },
}));

function request(
  overrides: Partial<ApprovalRequestItem> = {},
): ApprovalRequestItem {
  return {
    id: "req-pending",
    service_name: "Lark Bot",
    service_slug: "lark-bot",
    requester_type: "api_key",
    requester_label: "coding-agent",
    operation_summary: "POST /messages",
    action_description: "Post the drafted summary to #payments-oncall",
    approval_mode: "per_request",
    status: "pending",
    created_at: new Date(Date.now() - 60_000).toISOString(),
    expires_at: new Date(Date.now() + 14 * 60_000).toISOString(),
    decided_at: null,
    decision_channel: null,
    ...overrides,
  };
}

const PENDING = request();
const DECIDED: ApprovalRequestItem[] = [
  request({
    id: "req-approved",
    status: "approved",
    service_name: "GitHub",
    service_slug: "github",
    action_description: "Rotate the deploy key",
    decided_at: new Date(Date.now() - 30 * 60_000).toISOString(),
    decision_channel: "web",
  }),
  request({
    id: "req-denied",
    status: "rejected",
    action_description: "Share the weekly report by email",
    decided_at: new Date(Date.now() - 60 * 60_000).toISOString(),
    decision_channel: "push",
  }),
  request({
    id: "req-expired",
    status: "expired",
    action_description: "Share the weekly report to Lark",
    expires_at: new Date(Date.now() - 90 * 60_000).toISOString(),
  }),
];

function listResponse(requests: readonly ApprovalRequestItem[]) {
  return { requests, total: requests.length, page: 1, per_page: 50 };
}

function mockApi({ pending }: { readonly pending: ApprovalRequestItem[] }) {
  mockGet.mockImplementation((url: string) => {
    if (url.startsWith("/notifications/settings")) {
      return Promise.resolve({ grant_expiry_days: 7 });
    }
    if (url.includes("status=pending")) {
      return Promise.resolve(listResponse(pending));
    }
    return Promise.resolve(listResponse([...pending, ...DECIDED]));
  });
}

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

describe("ApprovalsView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockPost.mockResolvedValue({
      id: "req-pending",
      status: "approved",
      decided_at: new Date().toISOString(),
    });
  });

  it("lists pending approvals from the real listing with their service", async () => {
    mockApi({ pending: [PENDING] });
    render(<ApprovalsView />, { wrapper: createWrapper() });
    expect(await screen.findByText("Waiting on you")).toBeInTheDocument();
    expect(screen.getByText("Lark Bot")).toBeInTheDocument();
    expect(
      screen.getByText("Post the drafted summary to #payments-oncall"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /approve and send/i }),
    ).toBeInTheDocument();
    expect(mockGet).toHaveBeenCalledWith(
      "/approvals/requests?page=1&per_page=50&status=pending",
    );
  });

  it("renders decided requests in History and keeps pending ones out", async () => {
    mockApi({ pending: [PENDING] });
    render(<ApprovalsView />, { wrapper: createWrapper() });
    expect(await screen.findByText("History")).toBeInTheDocument();
    // Each entry renders twice: mobile card list + desktop table.
    expect(await screen.findAllByText("Approved")).toHaveLength(2);
    expect(screen.getAllByText("Denied")).toHaveLength(2);
    expect(screen.getAllByText("Expired")).toHaveLength(2);
    // NyxID "push" decisions surface as the mobile channel.
    expect(screen.getByText("Mobile")).toBeInTheDocument();
    // The pending request stays in the Waiting section only.
    expect(
      screen.getAllByText("Post the drafted summary to #payments-oncall"),
    ).toHaveLength(1);
  });

  it("decides a pending approval through the real endpoint", async () => {
    mockApi({ pending: [PENDING] });
    const user = userEvent.setup();
    render(<ApprovalsView />, { wrapper: createWrapper() });
    await user.click(
      await screen.findByRole("button", { name: /approve and send/i }),
    );
    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith(
        "/approvals/requests/req-pending/decide",
        { approved: true },
      );
    });
  });

  it("shows the empty state when nothing is pending", async () => {
    mockApi({ pending: [] });
    render(<ApprovalsView />, { wrapper: createWrapper() });
    expect(
      await screen.findByText("Nothing waiting on you"),
    ).toBeInTheDocument();
  });

  // A row missing a sort key used to throw out of `Array.sort`, and with no
  // router error component that unmounted the whole app — sidebar included —
  // so the page read as "every button is dead" rather than "one row is odd".
  it("still renders when a row is missing its sort timestamps", async () => {
    // Order matters: `Array.sort` only throws when the undated row lands on the
    // left of the comparison, so the undated row must not be first.
    const undated = request({
      id: "req-undated",
      action_description: "Post without an expiry",
      expires_at: undefined as unknown as string,
    });
    const undecided = request({
      id: "req-undecided",
      status: "approved",
      action_description: "Decided at an unknown time",
      created_at: undefined as unknown as string,
      decided_at: null,
    });
    mockGet.mockImplementation((url: string) => {
      if (url.startsWith("/notifications/settings")) {
        return Promise.resolve({ grant_expiry_days: 7 });
      }
      if (url.includes("status=pending")) {
        return Promise.resolve(listResponse([PENDING, undated]));
      }
      return Promise.resolve(listResponse([DECIDED[0]!, undecided]));
    });

    render(<ApprovalsView />, { wrapper: createWrapper() });

    expect(await screen.findByText("Waiting on you")).toBeInTheDocument();
    expect(screen.getByText("Post without an expiry")).toBeInTheDocument();
    expect(
      screen.getByText("Post the drafted summary to #payments-oncall"),
    ).toBeInTheDocument();
    expect(
      await screen.findAllByText("Decided at an unknown time"),
    ).toHaveLength(2);
    const historyRows = within(screen.getByRole("table")).getAllByRole("row");
    expect(historyRows[1]).toHaveTextContent("Rotate the deploy key");
    expect(historyRows[2]).toHaveTextContent("Decided at an unknown time");
  });
});

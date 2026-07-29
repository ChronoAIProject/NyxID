import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AdminAuditLogListResponse } from "@/types/admin";

const { mockNavigate, mockRefetch, mockUseAdminAuditLog, auditResponse, routeSearch } =
  vi.hoisted(() => {
    const auditResponse: AdminAuditLogListResponse = {
      entries: [
        {
          id: "aud-1",
          seq: 12,
          user_id: "11111111-1111-4111-8111-111111111111",
          api_key_id: "key-1",
          api_key_name: "claude-code-agent",
          event_type: "proxy.request",
          event_data: { response_status: 502 },
          ip_address: "10.0.1.50",
          user_agent: "nyxid-agent/0.9.2",
          created_at: "2026-07-12T09:25:00Z",
        },
        {
          id: "aud-2",
          seq: 11,
          user_id: null,
          api_key_id: null,
          api_key_name: null,
          event_type: "login",
          event_data: null,
          ip_address: null,
          user_agent: null,
          created_at: "2026-07-11T09:25:00Z",
        },
      ],
      total: 2,
      page: 1,
      per_page: 25,
      filter_options: {
        sorts: [
          "-created_at",
          "created_at",
          "event_type",
          "-event_type",
          "api_key_name",
          "-api_key_name",
          "api_key_id",
          "-api_key_id",
          "user_id",
          "-user_id",
          "ip_address",
          "-ip_address",
          "user_agent",
          "-user_agent",
          "status",
          "-status",
        ],
        search_fields: [
          { key: "event_type", label: "Event type" },
          { key: "user_id", label: "User ID" },
          { key: "api_key", label: "Agent / API key" },
          { key: "ip_address", label: "IP address" },
          { key: "user_agent", label: "User agent" },
        ],
        fields: [
          {
            key: "event_type",
            label: "Event type",
            value_type: "enum",
            operator: "is",
            multiple: true,
            supports_custom_text: true,
            options: [
              { value: "login", label: "login" },
              { value: "proxy.request", label: "proxy.request" },
            ],
          },
          {
            key: "status",
            label: "Status",
            value_type: "enum",
            operator: "is",
            multiple: true,
            supports_custom_text: false,
            options: [
              { value: "2xx", label: "2xx Success" },
              { value: "4xx", label: "4xx Client error" },
              { value: "5xx", label: "5xx Server error" },
              { value: "none", label: "No status" },
            ],
          },
          {
            key: "actor",
            label: "Actor",
            value_type: "enum",
            operator: "is",
            multiple: true,
            supports_custom_text: false,
            options: [
              { value: "user", label: "User session" },
              { value: "agent", label: "Agent API key" },
              { value: "anonymous", label: "Anonymous" },
            ],
          },
          {
            key: "created_at",
            label: "Created",
            value_type: "date",
            operator: "between",
            multiple: true,
            supports_custom_text: false,
            options: [],
          },
        ],
      },
    };
    return {
      mockNavigate: vi.fn(),
      mockRefetch: vi.fn(),
      mockUseAdminAuditLog: vi.fn(),
      auditResponse,
      routeSearch: {} as Record<string, unknown>,
    };
  });

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useSearch: () => routeSearch,
}));

vi.mock("@/hooks/use-admin", () => ({
  useAdminAuditLog: mockUseAdminAuditLog,
}));

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}));

import { AdminAuditLogPage } from "./admin-audit-log";

function resolveLastNavigationSearch(previous: Record<string, unknown> = {}) {
  const navigation = mockNavigate.mock.calls.at(-1)?.[0] as {
    readonly to: string;
    readonly search: (current: Record<string, unknown>) => unknown;
  };
  expect(navigation.to).toBe("/admin/audit-log");
  return navigation.search(previous);
}

beforeEach(() => {
  vi.clearAllMocks();
  for (const key of Object.keys(routeSearch)) delete routeSearch[key];
  mockUseAdminAuditLog.mockReturnValue({
    data: auditResponse,
    isLoading: false,
    isFetching: false,
    isPlaceholderData: false,
    error: null,
    refetch: mockRefetch,
  });
});

describe("AdminAuditLogPage", () => {
  it("renders audit entries with agent, status, and identity columns", () => {
    render(<AdminAuditLogPage />);

    expect(screen.getByText("proxy.request")).toBeInTheDocument();
    expect(screen.getByText("claude-code-agent")).toBeInTheDocument();
    expect(screen.getByText("502")).toBeInTheDocument();
    expect(
      screen.getByText("11111111-1111-4111-8111-111111111111"),
    ).toBeInTheDocument();
    expect(screen.getByText("Showing 1-2 of 2 events")).toBeInTheDocument();
  });

  it("no longer renders the standalone user-id / api-key-id search form", () => {
    render(<AdminAuditLogPage />);

    expect(
      screen.queryByPlaceholderText("Filter by user ID"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByPlaceholderText("Filter by API key ID"),
    ).not.toBeInTheDocument();
    // The scoped search box replaces it.
    expect(
      screen.getByLabelText("Search audit log"),
    ).toBeInTheDocument();
  });

  it("defaults to newest-first and sorts a column on click", async () => {
    const user = userEvent.setup();
    render(<AdminAuditLogPage />);

    const created = screen.getByRole("columnheader", { name: "Created" });
    expect(created).toHaveAttribute("aria-sort", "descending");

    await user.click(
      screen.getByRole("button", { name: "Sort by Event, ascending" }),
    );
    expect(resolveLastNavigationSearch()).toEqual({ sort: "event_type" });
  });

  it("leads the status column with the most severe responses", async () => {
    const user = userEvent.setup();
    render(<AdminAuditLogPage />);

    await user.click(
      screen.getByRole("button", { name: "Sort by Status, descending" }),
    );
    expect(resolveLastNavigationSearch()).toEqual({ sort: "-status" });
  });

  it("drops the sort param when a column returns to the default", async () => {
    const user = userEvent.setup();
    routeSearch.sort = "created_at";
    render(<AdminAuditLogPage />);

    await user.click(
      screen.getByRole("button", { name: "Sort by Created, descending" }),
    );
    expect(
      resolveLastNavigationSearch({ sort: "created_at" }),
    ).toEqual({});
  });

  it("pages forward and resets the page when the page size changes", async () => {
    const user = userEvent.setup();
    // More rows than one page holds, so "Next page" is reachable.
    mockUseAdminAuditLog.mockReturnValue({
      data: { ...auditResponse, total: 60 },
      isLoading: false,
      isFetching: false,
      isPlaceholderData: false,
      error: null,
      refetch: mockRefetch,
    });
    render(<AdminAuditLogPage />);

    await user.click(screen.getByRole("button", { name: "Next page" }));
    expect(resolveLastNavigationSearch()).toEqual({ page: 2 });

    await user.click(screen.getByRole("combobox", { name: "Rows per page" }));
    await user.click(screen.getByRole("option", { name: "50 rows" }));
    expect(resolveLastNavigationSearch({ page: 3 })).toEqual({ per_page: 50 });
  });

  it("searches a scoped field and shows it as a chip", async () => {
    const user = userEvent.setup();
    render(<AdminAuditLogPage />);

    await user.click(screen.getByRole("combobox", { name: "Search field" }));
    await user.click(screen.getByRole("option", { name: "User ID" }));
    await user.type(screen.getByLabelText("Search audit log"), "u1{Enter}");

    expect(resolveLastNavigationSearch()).toEqual({
      search_filters: '[{"field":"user_id","values":["u1"]}]',
    });
  });

  it("applies event-type and status filters together", async () => {
    const user = userEvent.setup();
    render(<AdminAuditLogPage />);

    await user.click(screen.getByRole("button", { name: "Filters" }));
    await user.click(screen.getByRole("checkbox", { name: "proxy.request" }));
    await user.click(screen.getByRole("button", { name: "Status" }));
    await user.click(
      screen.getByRole("checkbox", { name: "5xx Server error" }),
    );
    await user.click(screen.getByRole("button", { name: "Apply filters" }));

    expect(resolveLastNavigationSearch()).toEqual({
      event_type: "proxy.request",
      status: "5xx",
    });
  });

  it("clears every applied control at once", async () => {
    const user = userEvent.setup();
    routeSearch.search = "mfa";
    routeSearch.status = "5xx";
    render(<AdminAuditLogPage />);

    await user.click(screen.getByRole("button", { name: "Clear filters" }));

    expect(
      resolveLastNavigationSearch({ search: "mfa", status: "5xx" }),
    ).toEqual({});
  });

  it("offers a filter-aware empty state", async () => {
    const user = userEvent.setup();
    routeSearch.status = "5xx";
    mockUseAdminAuditLog.mockReturnValue({
      data: { ...auditResponse, entries: [], total: 0 },
      isLoading: false,
      isFetching: false,
      isPlaceholderData: false,
      error: null,
      refetch: mockRefetch,
    });
    render(<AdminAuditLogPage />);

    expect(
      screen.getByText("No events match these filters"),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Clear search and filters" }),
    );
    expect(resolveLastNavigationSearch({ status: "5xx" })).toEqual({});
  });

  it("retries after a failed load", async () => {
    const user = userEvent.setup();
    mockUseAdminAuditLog.mockReturnValue({
      data: undefined,
      isLoading: false,
      isFetching: false,
      isPlaceholderData: false,
      error: new Error("boom"),
      refetch: mockRefetch,
    });
    render(<AdminAuditLogPage />);

    expect(screen.getByText("Failed to load audit log")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => {
      expect(mockRefetch).toHaveBeenCalledTimes(1);
    });
  });

  it("freezes columns through the clicked header", async () => {
    const user = userEvent.setup();
    render(<AdminAuditLogPage />);

    await user.click(
      screen.getByRole("button", { name: "Freeze columns through Event" }),
    );

    expect(screen.getByRole("columnheader", { name: "Event" })).toHaveAttribute(
      "data-frozen",
      "true",
    );
    expect(
      screen.getByRole("button", { name: "Unfreeze columns through Event" }),
    ).toBeInTheDocument();
  });

  it("reorders a column with the keyboard", async () => {
    const user = userEvent.setup();
    render(<AdminAuditLogPage />);

    const handle = screen.getByRole("button", { name: "Move Created column" });
    handle.focus();
    await user.keyboard("{ArrowRight}");

    const headers = screen
      .getAllByRole("columnheader")
      .map((header) => header.getAttribute("data-column"));
    expect(headers.slice(0, 2)).toEqual(["event_type", "created_at"]);
  });
});

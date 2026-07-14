import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { stickyColumnLeft } from "@/lib/data-table-columns";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import type {
  AdminOAuthClientListResponse,
  BrokerSettingsResponse,
} from "@/types/admin";

const {
  mockNavigate,
  mockRefetch,
  mockUseAdminOAuthClients,
  mockUpdateClient,
  mockUpdateSettings,
  mockUseBrokerSettings,
  clientsResponse,
  brokerSettings,
  routeSearch,
} = vi.hoisted(() => {
  const clientsResponse: AdminOAuthClientListResponse = {
    clients: [
      {
        id: "dcr-aevatar",
        client_name: "Aevatar",
        client_type: "public",
        created_by: "dynamic_registration",
        redirect_uris: ["https://aevatar.example/callback"],
        allowed_scopes: "openid urn:nyxid:scope:broker_binding",
        delegation_scopes: "",
        broker_capability_enabled: false,
        broker_capability_effective: true,
        broker_capability_source: "scope",
        revocation_webhook_url: null,
        is_active: true,
        client_secret: null,
        created_at: "2026-07-01T00:00:00Z",
      },
    ],
    total: 1,
    page: 1,
    per_page: 25,
    filter_options: {
      client_types: ["public", "confidential", "other"],
      creator_types: ["dynamic_registration", "system", "owned", "ownerless"],
      broker_filters: ["enabled", "disabled", "flag", "scope"],
      statuses: [true, false],
      allowed_scopes: [
        "openid",
        "offline_access",
        "urn:nyxid:scope:broker_binding",
      ],
      sorts: [
        "-created_at",
        "created_at",
        "client_name",
        "-client_name",
        "client_type",
        "-client_type",
        "created_by",
        "-created_by",
        "broker",
        "-broker",
        "allowed_scopes",
        "-allowed_scopes",
        "-is_active",
        "is_active",
      ],
      search_fields: [
        { key: "client", label: "Client" },
        { key: "client_type", label: "Client type" },
        { key: "created_by", label: "Created by" },
        { key: "allowed_scopes", label: "Allowed scopes" },
      ],
      fields: [
        {
          key: "is_active",
          label: "Lifecycle status",
          value_type: "boolean",
          operator: "is",
          multiple: true,
          options: [
            { value: "true", label: "Active" },
            { value: "false", label: "Inactive" },
          ],
        },
        {
          key: "client_type",
          label: "Client type",
          value_type: "enum",
          operator: "is",
          multiple: true,
          options: [
            { value: "public", label: "Public" },
            { value: "confidential", label: "Confidential" },
            { value: "other", label: "Other" },
          ],
          supports_custom_text: true,
        },
        {
          key: "creator_type",
          label: "Creator",
          value_type: "enum",
          operator: "is",
          multiple: true,
          options: [
            {
              value: "dynamic_registration",
              label: "Dynamic registration",
            },
            { value: "system", label: "System" },
            { value: "owned", label: "User / org" },
            { value: "ownerless", label: "Ownerless" },
          ],
        },
        {
          key: "broker",
          label: "Broker capability",
          value_type: "enum",
          operator: "is",
          multiple: true,
          options: [
            { value: "enabled", label: "Enabled" },
            { value: "disabled", label: "Disabled" },
            { value: "flag", label: "Enabled by admin grant" },
            { value: "scope", label: "Enabled by broker scope" },
          ],
        },
        {
          key: "scope",
          label: "Allowed scope",
          value_type: "enum",
          operator: "includes",
          multiple: true,
          options: [
            { value: "openid", label: "OpenID" },
            { value: "offline_access", label: "Offline access" },
            {
              value: "urn:nyxid:scope:broker_binding",
              label: "Broker binding (server label)",
            },
          ],
        },
        {
          key: "created_at",
          label: "Created",
          value_type: "date",
          operator: "between",
          multiple: true,
          options: [],
        },
      ],
    },
  };
  const brokerSettings: BrokerSettingsResponse = {
    broker_require_sender_constraint: {
      effective: true,
      env_default: false,
      override: true,
      source: "override",
    },
    broker_require_admin_capability: {
      effective: false,
      env_default: false,
      override: null,
      source: "env_default",
    },
  };
  return {
    mockNavigate: vi.fn(),
    mockRefetch: vi.fn(),
    mockUseAdminOAuthClients: vi.fn(),
    mockUpdateClient: vi.fn(),
    mockUpdateSettings: vi.fn(),
    mockUseBrokerSettings: vi.fn(),
    clientsResponse,
    brokerSettings,
    routeSearch: {} as Record<string, unknown>,
  };
});

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useSearch: () => routeSearch,
}));

vi.mock("@/hooks/use-admin-oauth-clients", () => ({
  useAdminOAuthClients: mockUseAdminOAuthClients,
  useBrokerSettings: mockUseBrokerSettings,
  useUpdateAdminOAuthClient: () => ({
    mutateAsync: mockUpdateClient,
    isPending: false,
  }),
  useUpdateBrokerSettings: () => ({
    mutateAsync: mockUpdateSettings,
    isPending: false,
  }),
}));

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}));

import { AdminOAuthClientsPage } from "./admin-oauth-clients";

function adminUser(): User {
  return {
    id: "admin-1",
    email: "admin@example.com",
    display_name: "Admin",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: true,
    role: "admin",
    is_active: true,
    created_at: "2026-01-01T00:00:00Z",
  };
}

function operatorUser(): User {
  return {
    ...adminUser(),
    id: "operator-1",
    email: "operator@example.com",
    display_name: "Operator",
    is_admin: false,
    is_operator: true,
    role: "operator",
  };
}

function resolveLastNavigationSearch(previous: Record<string, unknown> = {}) {
  const navigation = mockNavigate.mock.calls.at(-1)?.[0] as {
    readonly to: string;
    readonly search: (current: Record<string, unknown>) => unknown;
  };
  expect(navigation.to).toBe("/admin/oauth-clients");
  return navigation.search(previous);
}

beforeEach(() => {
  vi.clearAllMocks();
  // The column layout persists, so a test that reorders or resizes would
  // otherwise seed the layout of every test that runs after it.
  localStorage.clear();
  for (const key of Object.keys(routeSearch)) delete routeSearch[key];
  mockUseAdminOAuthClients.mockReturnValue({
    data: clientsResponse,
    isLoading: false,
    isFetching: false,
    isPlaceholderData: false,
    error: null,
    refetch: mockRefetch,
  });
  mockUpdateClient.mockResolvedValue(clientsResponse.clients[0]);
  mockUpdateSettings.mockResolvedValue(brokerSettings);
  mockUseBrokerSettings.mockReturnValue({
    data: brokerSettings,
    isLoading: false,
    error: null,
  });
  useAuthStore.setState({
    user: adminUser(),
    isAuthenticated: true,
    isLoading: false,
  });
});

describe("AdminOAuthClientsPage", () => {
  it("renders all clients including dynamic-registration clients", () => {
    render(<AdminOAuthClientsPage />);

    expect(screen.getByText("Aevatar")).toBeInTheDocument();
    expect(screen.getByText("dcr-aevatar")).toBeInTheDocument();
    expect(screen.getByText("dynamic_registration")).toBeInTheDocument();
    expect(
      screen.getByText("urn:nyxid:scope:broker_binding"),
    ).toBeInTheDocument();
    expect(screen.getByText("Showing 1-1 of 1 clients")).toBeInTheDocument();
    expect(screen.getByRole("table").parentElement).toHaveClass(
      "overflow-auto",
      "overscroll-x-none",
    );
    expect(mockUseAdminOAuthClients).toHaveBeenCalledWith({
      page: 1,
      per_page: 25,
      search: undefined,
      search_filters: undefined,
      client_type: undefined,
      creator_type: undefined,
      broker: undefined,
      is_active: undefined,
      scope: undefined,
      created_dates: undefined,
      created_from: undefined,
      created_to: undefined,
      sort: "-created_at",
    });
  });

  it("applies search through URL state and resets the page", async () => {
    const user = userEvent.setup();
    routeSearch.page = 4;
    render(<AdminOAuthClientsPage />);

    await user.type(
      screen.getByLabelText("Search OAuth clients"),
      " console {Enter}",
    );

    const navigation = mockNavigate.mock.calls.at(-1)?.[0] as {
      readonly to: string;
      readonly search: (previous: Record<string, unknown>) => unknown;
    };
    expect(navigation.to).toBe("/admin/oauth-clients");
    expect(navigation.search({ page: 4 })).toEqual({ search: "console" });
    expect(
      screen.queryByRole("button", { name: "Apply search" }),
    ).not.toBeInTheDocument();
  });

  it("clears an unsubmitted search draft across browser history changes", async () => {
    const user = userEvent.setup();
    routeSearch.search = "existing";
    const { rerender } = render(<AdminOAuthClientsPage />);

    const input = screen.getByLabelText("Search OAuth clients");
    expect(input).toHaveValue("");
    await user.type(input, "unsubmitted");

    routeSearch.search = "history-value";
    rerender(<AdminOAuthClientsPage />);
    await waitFor(() =>
      expect(screen.getByLabelText("Search OAuth clients")).toHaveValue(""),
    );
  });

  it("applies typed search only after Enter or leaving the search control", async () => {
    const user = userEvent.setup();
    routeSearch.page = 3;
    render(<AdminOAuthClientsPage />);
    mockNavigate.mockClear();

    const input = screen.getByLabelText("Search OAuth clients");
    await user.type(input, "console");

    expect(mockNavigate).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Filters" }));
    expect(mockNavigate).toHaveBeenCalledOnce();
    expect(input).not.toHaveFocus();
    expect(resolveLastNavigationSearch({ page: 3 })).toEqual({
      search: "console",
    });
  });

  it("shows exactly one applied search filter and lets users edit or remove it", async () => {
    const user = userEvent.setup();
    routeSearch.search = "console";
    routeSearch.sort = "client_name";
    render(<AdminOAuthClientsPage />);

    const searchFilterChip = screen.getByRole("button", {
      name: "Edit All fields search: contains console",
    });
    expect(
      screen.getAllByRole("button", {
        name: "Edit All fields search: contains console",
      }),
    ).toHaveLength(1);
    expect(searchFilterChip).toHaveTextContent('All fields contains "console"');
    expect(
      screen.getByRole("button", { name: "Filters, 1 applied" }),
    ).toBeInTheDocument();

    await user.click(searchFilterChip);
    expect(screen.getByLabelText("Search OAuth clients")).toHaveFocus();

    await user.click(
      screen.getByRole("button", { name: "Remove All fields search" }),
    );
    expect(
      resolveLastNavigationSearch({
        page: 2,
        search: "console",
        sort: "client_name",
      }),
    ).toEqual({ sort: "client_name" });
  });

  it("adds scoped search values on Enter and ORs values in one column", async () => {
    const user = userEvent.setup();
    routeSearch.page = 3;
    routeSearch.search_filters = JSON.stringify([
      { field: "client", values: ["console"] },
    ]);
    render(<AdminOAuthClientsPage />);
    mockNavigate.mockClear();

    await user.click(screen.getByRole("combobox", { name: "Search field" }));
    await user.click(screen.getByRole("option", { name: "Client" }));
    await user.type(screen.getByLabelText("Search OAuth clients"), "portal");
    expect(mockNavigate).not.toHaveBeenCalled();
    await user.keyboard("{Enter}");

    expect(
      resolveLastNavigationSearch({
        page: 3,
        search_filters: JSON.stringify([
          { field: "client", values: ["console"] },
        ]),
      }),
    ).toEqual({
      search_filters: JSON.stringify([
        { field: "client", values: ["console", "portal"] },
      ]),
    });
    expect(screen.getByLabelText("Search OAuth clients")).toHaveValue("");
    expect(screen.getByLabelText("Search OAuth clients")).toHaveFocus();
    expect(
      screen.getByRole("combobox", { name: "Search field" }),
    ).toHaveTextContent("In: All fields");
  });

  it("renders OR within a search column without inter-chip operators", async () => {
    const user = userEvent.setup();
    routeSearch.search_filters = JSON.stringify([
      { field: "client", values: ["console", "portal"] },
      { field: "created_by", values: ["dynamic"] },
    ]);
    render(<AdminOAuthClientsPage />);

    expect(
      screen.getByRole("group", {
        name: "Client search, matches any term",
      }),
    ).toHaveTextContent(/Client contains.*"console".*OR.*"portal"/);
    expect(
      screen.getByRole("group", {
        name: "Created by search, matches any term",
      }),
    ).toHaveTextContent(/Created by contains.*"dynamic"/);
    expect(screen.getAllByText("OR")).toHaveLength(1);
    expect(screen.queryByText("AND")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Filters, 2 applied" }),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", {
        name: "Remove Client search value: portal",
      }),
    );
    expect(
      resolveLastNavigationSearch({
        search_filters: JSON.stringify([
          { field: "client", values: ["console", "portal"] },
          { field: "created_by", values: ["dynamic"] },
        ]),
      }),
    ).toEqual({
      search_filters: JSON.stringify([
        { field: "client", values: ["console"] },
        { field: "created_by", values: ["dynamic"] },
      ]),
    });
  });

  it("does not apply a draft when focus moves to the field dropdown", async () => {
    const user = userEvent.setup();
    render(<AdminOAuthClientsPage />);
    const input = screen.getByLabelText("Search OAuth clients");
    await user.type(input, "dynamic");

    await user.click(screen.getByRole("combobox", { name: "Search field" }));
    expect(mockNavigate).not.toHaveBeenCalled();
    await user.click(screen.getByRole("option", { name: "Created by" }));
    expect(mockNavigate).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Filters" }));
    expect(mockNavigate).toHaveBeenCalledOnce();
    expect(
      screen.getByRole("combobox", { name: "Search field" }),
    ).toHaveTextContent("In: Created by");
  });

  it("stages multiple metadata-backed values and applies them through URL state", async () => {
    const user = userEvent.setup();
    routeSearch.page = 4;
    routeSearch.search = "aevatar";
    routeSearch.sort = "client_name";
    render(<AdminOAuthClientsPage />);
    mockNavigate.mockClear();

    await user.click(
      screen.getByRole("button", { name: "Filters, 1 applied" }),
    );

    expect(
      screen.getByRole("button", { name: "Lifecycle status" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("checkbox", { name: "Active" }));

    await user.click(screen.getByRole("button", { name: "Allowed scope" }));
    const brokerBindingScope = screen.getByRole("checkbox", {
      name: "Broker binding (server label)",
    });
    expect(brokerBindingScope).toBeInTheDocument();
    await user.click(brokerBindingScope);

    await user.click(screen.getByRole("button", { name: "Client type" }));
    await user.click(screen.getByRole("checkbox", { name: "Public" }));
    await user.click(screen.getByRole("checkbox", { name: "Confidential" }));

    await user.click(screen.getByRole("button", { name: /^Lifecycle status/ }));
    expect(screen.getByRole("checkbox", { name: "Active" })).toBeChecked();

    expect(mockNavigate).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Apply filters" }));

    expect(
      resolveLastNavigationSearch({
        page: 4,
        search: "aevatar",
        sort: "client_name",
      }),
    ).toEqual({
      search: "aevatar",
      client_type: "public,confidential",
      is_active: true,
      scope: "urn:nyxid:scope:broker_binding",
      sort: "client_name",
    });
  });

  it("aggregates multi-value chips and lets users edit and uncheck one option", async () => {
    const user = userEvent.setup();
    routeSearch.search = "aevatar";
    routeSearch.is_active = "true,false";
    routeSearch.client_type = "public,confidential,other";
    routeSearch.scope = "openid,urn:nyxid:scope:broker_binding";
    routeSearch.sort = "client_name";
    render(<AdminOAuthClientsPage />);

    expect(
      screen.getByRole("button", {
        name: /^Edit Lifecycle status filter:/,
      }),
    ).toHaveTextContent("Lifecycle status is any of Active, Inactive");
    expect(
      screen.getByRole("button", { name: /^Edit Client type filter:/ }),
    ).toHaveTextContent("Client type is any of Public, Confidential +1 more");
    expect(
      screen.getByRole("button", { name: /^Edit Allowed scope filter:/ }),
    ).toHaveTextContent(
      "Allowed scope includes any of OpenID, Broker binding (server label)",
    );
    expect(
      screen.getByRole("button", { name: "Filters, 4 applied" }),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: /^Edit Client type filter:/ }),
    );
    expect(screen.getByRole("checkbox", { name: "Public" })).toBeChecked();
    expect(
      screen.getByRole("checkbox", { name: "Confidential" }),
    ).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Other" })).toBeChecked();

    await user.click(screen.getByRole("checkbox", { name: "Public" }));
    expect(screen.getByRole("checkbox", { name: "Public" })).not.toBeChecked();
    expect(mockNavigate).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Apply filters" }));

    expect(
      resolveLastNavigationSearch({
        page: 3,
        search: "aevatar",
        is_active: "true,false",
        client_type: "public,confidential,other",
        scope: "openid,urn:nyxid:scope:broker_binding",
        sort: "client_name",
      }),
    ).toEqual({
      search: "aevatar",
      is_active: "true,false",
      client_type: "confidential,other",
      scope: "openid,urn:nyxid:scope:broker_binding",
      sort: "client_name",
    });
  });

  it("removes one multi-value filter without changing search, sort, or other filters", async () => {
    const user = userEvent.setup();
    routeSearch.search = "aevatar";
    routeSearch.is_active = "true,false";
    routeSearch.client_type = "public,confidential";
    routeSearch.sort = "client_name";
    render(<AdminOAuthClientsPage />);

    await user.click(
      screen.getByRole("button", {
        name: "Remove Lifecycle status filter",
      }),
    );

    expect(
      resolveLastNavigationSearch({
        page: 3,
        search: "aevatar",
        is_active: "true,false",
        client_type: "public,confidential",
        sort: "client_name",
      }),
    ).toEqual({
      search: "aevatar",
      client_type: "public,confidential",
      sort: "client_name",
    });
  });

  it("restores date bounds without showing inter-chip operators", async () => {
    const user = userEvent.setup();
    routeSearch.search = "public";
    routeSearch.client_type = "public";
    routeSearch.created_from = "2026-07-01";
    routeSearch.created_to = "2026-07-31";
    render(<AdminOAuthClientsPage />);

    expect(mockUseAdminOAuthClients).toHaveBeenCalledWith(
      expect.objectContaining({
        search: "public",
        client_type: "public",
        created_from: "2026-07-01",
        created_to: "2026-07-31",
      }),
    );
    expect(
      screen.getByRole("button", { name: /^Edit Created filter:/ }),
    ).toHaveTextContent("Created is between Jul 1, 2026 and Jul 31, 2026");
    expect(screen.queryByText("AND")).not.toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Remove Created filter" }),
    );
    expect(
      resolveLastNavigationSearch({
        page: 2,
        search: "public",
        client_type: "public",
        created_from: "2026-07-01",
        created_to: "2026-07-31",
      }),
    ).toEqual({
      search: "public",
      client_type: "public",
    });
  });

  it("restores and removes an exact multi-date filter", async () => {
    const user = userEvent.setup();
    routeSearch.created_dates = "2026-07-03,2026-07-08,2026-07-21";
    render(<AdminOAuthClientsPage />);

    expect(mockUseAdminOAuthClients).toHaveBeenCalledWith(
      expect.objectContaining({
        created_dates: "2026-07-03,2026-07-08,2026-07-21",
        created_from: undefined,
        created_to: undefined,
      }),
    );
    expect(
      screen.getByRole("button", { name: /^Edit Created filter:/ }),
    ).toHaveTextContent(
      "Created is on any of Jul 3, 2026, Jul 8, 2026 +1 more",
    );

    await user.click(
      screen.getByRole("button", { name: "Remove Created filter" }),
    );
    expect(
      resolveLastNavigationSearch({
        page: 2,
        created_dates: "2026-07-03,2026-07-08,2026-07-21",
      }),
    ).toEqual({});
  });

  it("clears search and structured filters together", async () => {
    const user = userEvent.setup();
    routeSearch.search = "console";
    routeSearch.broker = "enabled,scope";
    routeSearch.scope = "openid,urn:nyxid:scope:broker_binding";
    routeSearch.created_from = "2026-07-01";
    routeSearch.created_to = "2026-07-31";
    routeSearch.sort = "-client_name";
    render(<AdminOAuthClientsPage />);

    await user.click(screen.getByRole("button", { name: "Clear filters" }));

    expect(
      resolveLastNavigationSearch({
        page: 2,
        search: "console",
        broker: "enabled,scope",
        scope: "openid,urn:nyxid:scope:broker_binding",
        created_from: "2026-07-01",
        created_to: "2026-07-31",
        sort: "-client_name",
      }),
    ).toEqual({
      sort: "-client_name",
    });
  });

  it("sorts the complete dataset through each column header", async () => {
    const user = userEvent.setup();
    const { rerender } = render(<AdminOAuthClientsPage />);

    for (const label of [
      "Client",
      "Type",
      "Created By",
      "Broker",
      "Status",
      "Allowed Scopes",
      "Created",
    ]) {
      expect(
        screen.getByRole("button", {
          name: new RegExp(`^Sort by ${label},`),
        }),
      ).toBeEnabled();
    }

    expect(
      screen.getByRole("columnheader", { name: "Created" }),
    ).toHaveAttribute("aria-sort", "descending");
    expect(
      screen.queryByLabelText("Sort OAuth clients"),
    ).not.toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Sort by Client, ascending" }),
    );
    expect(resolveLastNavigationSearch({ page: 5 })).toEqual({
      sort: "client_name",
    });

    routeSearch.sort = "client_name";
    rerender(<AdminOAuthClientsPage />);
    await user.click(
      screen.getByRole("button", { name: "Sort by Client, descending" }),
    );
    expect(resolveLastNavigationSearch({ sort: "client_name" })).toEqual({
      sort: "-client_name",
    });

    routeSearch.sort = undefined;
    rerender(<AdminOAuthClientsPage />);
    await user.click(
      screen.getByRole("button", { name: "Sort by Broker, descending" }),
    );
    expect(resolveLastNavigationSearch()).toEqual({ sort: "-broker" });
  });

  it("pages by 10 rows and keeps the default page size out of the URL", async () => {
    const user = userEvent.setup();
    routeSearch.page = 4;
    const { rerender } = render(<AdminOAuthClientsPage />);

    await user.click(screen.getByRole("combobox", { name: "Rows per page" }));
    await user.click(screen.getByRole("option", { name: "10 rows" }));
    expect(resolveLastNavigationSearch({ page: 4 })).toEqual({ per_page: 10 });

    routeSearch.per_page = 10;
    rerender(<AdminOAuthClientsPage />);

    await user.click(screen.getByRole("combobox", { name: "Rows per page" }));
    await user.click(screen.getByRole("option", { name: "25 rows" }));
    expect(resolveLastNavigationSearch({ per_page: 10 })).toEqual({});
  });

  it("reorders headers and row cells from a dedicated drag handle", () => {
    render(<AdminOAuthClientsPage />);

    const expectedLabels = [
      "Client",
      "Type",
      "Created By",
      "Broker",
      "Status",
      "Allowed Scopes",
      "Created",
    ];
    expect(
      screen
        .getAllByRole("columnheader")
        .map((header) => header.getAttribute("aria-label")),
    ).toEqual(expectedLabels);
    for (const label of expectedLabels) {
      expect(
        screen.getByRole("button", { name: `Move ${label} column` }),
      ).toBeInTheDocument();
    }
    expect(screen.getByRole("columnheader", { name: "Client" })).toHaveClass(
      "p-0",
      "group/header",
    );

    const source = screen.getByRole("button", {
      name: "Move Client column",
    });
    expect(source).toHaveClass(
      "w-7",
      "opacity-0",
      "group-hover/header:opacity-100",
    );
    const target = screen.getByRole("columnheader", { name: "Status" });
    vi.spyOn(target, "getBoundingClientRect").mockReturnValue({
      x: 0,
      y: 0,
      width: 200,
      height: 40,
      top: 0,
      right: 200,
      bottom: 40,
      left: 0,
      toJSON: () => ({}),
    });
    let transferredColumn = "";
    const dataTransfer = {
      effectAllowed: "none",
      dropEffect: "none",
      setData: (_type: string, value: string) => {
        transferredColumn = value;
      },
      getData: () => transferredColumn,
    } as unknown as DataTransfer;

    fireEvent.dragStart(source, { dataTransfer });
    fireEvent.dragOver(target, { clientX: 160, dataTransfer });
    expect(target).toHaveAttribute("data-drop-position", "after");
    expect(screen.getByTestId("column-drop-indicator-is_active")).toHaveClass(
      "inset-y-0",
      "right-0",
    );
    fireEvent.drop(target, { clientX: 160, dataTransfer });

    expect(
      screen
        .getAllByRole("columnheader")
        .map((header) => header.getAttribute("aria-label")),
    ).toEqual([
      "Type",
      "Created By",
      "Broker",
      "Status",
      "Client",
      "Allowed Scopes",
      "Created",
    ]);
    const dataCells = screen
      .getByRole("table")
      .querySelectorAll<HTMLTableCellElement>("tbody td");
    expect(dataCells[0]).toHaveTextContent("public");
    expect(dataCells[4]).toHaveTextContent("Aevatar");
    expect(
      screen.getByText("Client column moved to position 5 of 7"),
    ).toBeInTheDocument();
  });

  it("reorders columns from the keyboard without breaking header sorting", async () => {
    const user = userEvent.setup();
    render(<AdminOAuthClientsPage />);

    const clientHandle = screen.getByRole("button", {
      name: "Move Client column",
    });
    clientHandle.focus();
    await user.keyboard("{ArrowRight}");

    expect(
      screen
        .getAllByRole("columnheader")
        .slice(0, 2)
        .map((header) => header.getAttribute("aria-label")),
    ).toEqual(["Type", "Client"]);
    expect(
      screen.getByText("Client column moved to position 2 of 7"),
    ).toBeInTheDocument();

    await user.keyboard("{End}");
    expect(screen.getAllByRole("columnheader").at(-1)).toHaveAccessibleName(
      "Client",
    );
    expect(
      screen.getByText("Client column moved to position 7 of 7"),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Sort by Client, ascending" }),
    );
    expect(resolveLastNavigationSearch({ page: 3 })).toEqual({
      sort: "client_name",
    });
  });

  it("freezes through a column and recomputes sticky offsets after reordering", async () => {
    const user = userEvent.setup();
    render(<AdminOAuthClientsPage />);

    await user.click(
      screen.getByRole("button", {
        name: "Freeze columns through Created By",
      }),
    );

    const clientHeader = screen.getByRole("columnheader", { name: "Client" });
    const typeHeader = screen.getByRole("columnheader", { name: "Type" });
    const creatorHeader = screen.getByRole("columnheader", {
      name: "Created By",
    });
    const table = screen.getByRole("table");
    expect(clientHeader).toHaveAttribute("data-frozen", "true");
    expect(typeHeader).toHaveAttribute("data-frozen", "true");
    expect(creatorHeader).toHaveAttribute("data-frozen", "true");
    expect(clientHeader).toHaveClass("z-30", "bg-card");
    // Widths live in a CSS variable per column; the frozen offsets are sums of
    // those variables, unit-tested in lib/data-table-columns.test.ts (happy-dom
    // drops `calc(var(...))` from a length, so it can't be read back here).
    expect(table.style.getPropertyValue("--col-w-client_name")).toBe("260px");
    expect(table.style.getPropertyValue("--col-w-client_type")).toBe("140px");
    expect(clientHeader).toHaveStyle({ left: "0px" });
    // The frozen edge has to be painted by the sticky cell itself. A collapsed
    // `border-r` belongs to the table's border grid, so it would slide out from
    // under the frozen column as soon as the table scrolls horizontally.
    expect(creatorHeader).toHaveAttribute("data-frozen-edge", "true");
    expect(creatorHeader).not.toHaveClass("border-r-2");
    expect(creatorHeader).toHaveClass("before:w-0.5", "before:bg-border");
    expect(
      screen.getByRole("button", {
        name: "Unfreeze columns through Created By",
      }),
    ).toHaveClass("opacity-100", "text-primary");

    expect(
      table.querySelector('tbody td[data-column="client_name"]'),
    ).toHaveClass("sticky", "z-20", "bg-card");
    const edgeCell = table.querySelector('tbody td[data-column="created_by"]');
    expect(edgeCell).toHaveAttribute("data-frozen-edge", "true");
    expect(edgeCell).not.toHaveClass("border-r-2");
    expect(edgeCell).toHaveClass("before:w-0.5", "before:bg-border");

    const clientHandle = screen.getByRole("button", {
      name: "Move Client column",
    });
    clientHandle.focus();
    await user.keyboard("{ArrowRight}");

    expect(typeHeader).toHaveStyle({ left: "0px" });
    expect(clientHeader).toHaveAttribute("data-frozen", "true");
    expect(creatorHeader).toHaveAttribute("data-frozen", "true");

    await user.click(
      screen.getByRole("button", {
        name: "Unfreeze columns through Created By",
      }),
    );
    expect(clientHeader).not.toHaveAttribute("data-frozen");
    expect(typeHeader).not.toHaveAttribute("data-frozen");
    expect(creatorHeader).not.toHaveAttribute("data-frozen");
    expect(creatorHeader).not.toHaveAttribute("data-frozen-edge");
    expect(edgeCell).not.toHaveAttribute("data-frozen-edge");
    expect(
      table.querySelector('tbody td[data-column="client_name"]'),
    ).not.toHaveClass("sticky");
  });

  it("resizes a column by dragging its handle", () => {
    render(<AdminOAuthClientsPage />);

    const table = screen.getByRole("table");
    const handle = screen.getByRole("separator", {
      name: "Resize Client column",
    });

    fireEvent.pointerDown(handle, { button: 0, clientX: 260 });
    fireEvent.pointerMove(window, { clientX: 340 });
    // The drag writes straight to the width variable so the rows never re-render.
    expect(table.style.getPropertyValue("--col-w-client_name")).toBe("340px");

    fireEvent.pointerUp(window, { clientX: 340 });
    expect(table.style.getPropertyValue("--col-w-client_name")).toBe("340px");
    expect(handle).toHaveAttribute("aria-valuenow", "340");
    expect(
      screen.getByText("Client column resized to 340 pixels"),
    ).toBeInTheDocument();
  });

  it("tracks a drag that starts and ends before React can re-render", () => {
    render(<AdminOAuthClientsPage />);

    const table = screen.getByRole("table");
    const clientHeader = screen.getByRole("columnheader", { name: "Client" });
    const handle = screen.getByRole("separator", {
      name: "Resize Client column",
    });

    // A whole drag inside one task: the listeners have to be live from the
    // pointerdown, or the move is lost and the pointerup strands the table in
    // its resizing state.
    fireEvent.pointerDown(handle, { button: 0, clientX: 260 });
    fireEvent.pointerMove(window, { clientX: 330 });
    fireEvent.pointerUp(window, { clientX: 330 });

    expect(table.style.getPropertyValue("--col-w-client_name")).toBe("330px");
    expect(clientHeader).not.toHaveAttribute("data-resizing");
  });

  it("clamps a drag to the min and max column width", () => {
    render(<AdminOAuthClientsPage />);

    const table = screen.getByRole("table");
    const handle = screen.getByRole("separator", {
      name: "Resize Client column",
    });

    fireEvent.pointerDown(handle, { button: 0, clientX: 260 });
    fireEvent.pointerMove(window, { clientX: -500 });
    expect(table.style.getPropertyValue("--col-w-client_name")).toBe("96px");
    fireEvent.pointerMove(window, { clientX: 2000 });
    expect(table.style.getPropertyValue("--col-w-client_name")).toBe("640px");
    fireEvent.pointerUp(window, { clientX: 2000 });

    expect(handle).toHaveAttribute("aria-valuenow", "640");
  });

  it("resizes from the keyboard and restores the default width on double-click", async () => {
    const user = userEvent.setup();
    render(<AdminOAuthClientsPage />);

    const table = screen.getByRole("table");
    const handle = screen.getByRole("separator", {
      name: "Resize Client column",
    });

    handle.focus();
    await user.keyboard("{ArrowRight}{ArrowRight}");
    expect(table.style.getPropertyValue("--col-w-client_name")).toBe("292px");
    await user.keyboard("{ArrowLeft}");
    expect(table.style.getPropertyValue("--col-w-client_name")).toBe("276px");

    await user.dblClick(handle);
    expect(table.style.getPropertyValue("--col-w-client_name")).toBe("260px");
    expect(handle).toHaveAttribute("aria-valuenow", "260");
  });

  it("keeps a frozen column's offset in step with a resize of the column before it", () => {
    render(<AdminOAuthClientsPage />);

    fireEvent.click(
      screen.getByRole("button", { name: "Freeze columns through Type" }),
    );
    const table = screen.getByRole("table");
    const typeHeader = screen.getByRole("columnheader", { name: "Type" });
    expect(typeHeader).toHaveAttribute("data-frozen", "true");

    const handle = screen.getByRole("separator", {
      name: "Resize Client column",
    });
    fireEvent.pointerDown(handle, { button: 0, clientX: 260 });
    fireEvent.pointerMove(window, { clientX: 300 });
    fireEvent.pointerUp(window, { clientX: 300 });

    // Type's sticky offset resolves the same variable the drag rewrote, so it
    // stays pinned to Client's right edge instead of detaching mid-drag.
    expect(table.style.getPropertyValue("--col-w-client_name")).toBe("300px");
    expect(
      stickyColumnLeft(["client_name", "client_type"], "client_type"),
    ).toBe("calc(var(--col-w-client_name))");
  });

  it("persists a resize, a reorder and a freeze across a remount", () => {
    const { unmount } = render(<AdminOAuthClientsPage />);

    const handle = screen.getByRole("separator", {
      name: "Resize Client column",
    });
    fireEvent.pointerDown(handle, { button: 0, clientX: 260 });
    fireEvent.pointerMove(window, { clientX: 340 });
    fireEvent.pointerUp(window, { clientX: 340 });
    fireEvent.click(
      screen.getByRole("button", { name: "Freeze columns through Type" }),
    );
    const clientGrip = screen.getByRole("button", {
      name: "Move Client column",
    });
    clientGrip.focus();
    fireEvent.keyDown(clientGrip, { key: "ArrowRight" });

    expect(
      JSON.parse(
        localStorage.getItem("nyxid.table.admin-oauth-clients.columns.v1") ??
          "{}",
      ),
    ).toMatchObject({
      order: [
        "client_type",
        "client_name",
        "created_by",
        "broker",
        "is_active",
        "allowed_scopes",
        "created_at",
      ],
      frozenThrough: "client_type",
      widths: expect.objectContaining({ client_name: 340 }),
    });

    unmount();
    render(<AdminOAuthClientsPage />);

    const table = screen.getByRole("table");
    expect(table.style.getPropertyValue("--col-w-client_name")).toBe("340px");
    expect(
      [...table.querySelectorAll("thead th")].map((th) =>
        th.getAttribute("data-column"),
      ),
    ).toEqual([
      "client_type",
      "client_name",
      "created_by",
      "broker",
      "is_active",
      "allowed_scopes",
      "created_at",
    ]);
    expect(screen.getByRole("columnheader", { name: "Type" })).toHaveAttribute(
      "data-frozen-edge",
      "true",
    );
  });

  it("restores the defaults from the reset control and forgets the stored layout", () => {
    render(<AdminOAuthClientsPage />);

    expect(
      screen.queryByRole("button", { name: /Reset columns/ }),
    ).not.toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: "Freeze columns through Type" }),
    );
    expect(
      localStorage.getItem("nyxid.table.admin-oauth-clients.columns.v1"),
    ).not.toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Reset columns/ }));

    expect(
      screen.getByRole("columnheader", { name: "Type" }),
    ).not.toHaveAttribute("data-frozen");
    expect(
      screen.queryByRole("button", { name: /Reset columns/ }),
    ).not.toBeInTheDocument();
    expect(
      localStorage.getItem("nyxid.table.admin-oauth-clients.columns.v1"),
    ).toBeNull();
  });

  it("ignores a stored layout that no longer matches the table", () => {
    localStorage.setItem(
      "nyxid.table.admin-oauth-clients.columns.v1",
      JSON.stringify({
        order: ["client_secret", "client_name"],
        frozenThrough: "client_secret",
        widths: { client_name: 9000 },
      }),
    );

    render(<AdminOAuthClientsPage />);

    const table = screen.getByRole("table");
    // A column that no longer exists cannot survive as a freeze point, and a
    // width from other bounds falls back rather than rendering a broken table.
    expect(table.style.getPropertyValue("--col-w-client_name")).toBe("260px");
    expect(
      screen.getByRole("columnheader", { name: "Client" }),
    ).not.toHaveAttribute("data-frozen");
    expect(
      [...table.querySelectorAll("thead th")].map((th) =>
        th.getAttribute("data-column"),
      ),
    ).toEqual([
      "client_name",
      "client_type",
      "created_by",
      "broker",
      "is_active",
      "allowed_scopes",
      "created_at",
    ]);
  });

  it("sends a filter's custom text as a contains search beside its checked options", async () => {
    const user = userEvent.setup();
    render(<AdminOAuthClientsPage />);

    await user.click(screen.getByRole("button", { name: "Filters" }));
    await user.click(screen.getByRole("button", { name: /Client type/ }));
    await user.click(screen.getByRole("checkbox", { name: "Public" }));
    await user.type(
      screen.getByRole("textbox", { name: "Custom Client type value" }),
      "acme",
    );
    await user.click(screen.getByRole("button", { name: "Add" }));
    await user.click(screen.getByRole("button", { name: "Apply filters" }));

    expect(resolveLastNavigationSearch({ page: 4 })).toEqual({
      client_type: "public",
      custom_filters: '{"client_type":["acme"]}',
    });
  });

  it("clears a filter's custom text without dropping its checked options", async () => {
    routeSearch.client_type = "public";
    routeSearch.custom_filters = '{"client_type":["acme"]}';
    const user = userEvent.setup();
    render(<AdminOAuthClientsPage />);

    // The custom text is its own chip, and reads as a contains.
    expect(
      screen.getByRole("button", {
        name: "Edit Client type custom text: contains acme",
      }),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Remove Client type custom text" }),
    );
    expect(resolveLastNavigationSearch(routeSearch)).toEqual({
      client_type: "public",
    });
  });

  it("keeps frozen cells opaque and leaves no highlight behind after a drop", async () => {
    const user = userEvent.setup();
    render(<AdminOAuthClientsPage />);

    await user.click(
      screen.getByRole("button", { name: "Freeze columns through Type" }),
    );

    // A frozen cell must never swap its opaque background for the translucent
    // accent/muted tints, or the columns scrolled underneath bleed through it.
    const typeHeader = screen.getByRole("columnheader", { name: "Type" });
    expect(typeHeader).toHaveClass("bg-card");
    expect(typeHeader).not.toHaveClass(
      "hover:bg-accent",
      "focus-within:bg-accent",
    );
    const typeCell = screen
      .getByRole("table")
      .querySelector('tbody td[data-column="client_type"]');
    expect(typeCell).toHaveClass("bg-card");
    expect(typeCell).not.toHaveClass("group-hover/row:bg-muted");

    // The header tint is a hover-only overlay, so focus left on the pin button
    // after a click does not strand the column in a highlighted state.
    const tint = typeHeader.querySelector("span[aria-hidden]");
    expect(tint).toHaveClass("opacity-0", "group-hover/header:opacity-100");

    const source = screen.getByRole("button", { name: "Move Client column" });
    const target = screen.getByRole("columnheader", { name: "Status" });
    let transferredColumn = "";
    const dataTransfer = {
      effectAllowed: "none",
      dropEffect: "none",
      setData: (_type: string, value: string) => {
        transferredColumn = value;
      },
      getData: () => transferredColumn,
    } as unknown as DataTransfer;

    fireEvent.dragStart(source, { dataTransfer });
    fireEvent.dragOver(target, { clientX: 160, dataTransfer });
    fireEvent.drop(target, { clientX: 160, dataTransfer });

    for (const header of screen.getAllByRole("columnheader")) {
      expect(header).not.toHaveAttribute("data-dragging");
      expect(header).not.toHaveAttribute("data-drop-position");
    }
    expect(
      screen.queryByTestId("column-drop-indicator-is_active"),
    ).not.toBeInTheDocument();
  });

  it("marks retained rows as updating and prevents stale row mutations", () => {
    routeSearch.page = 2;
    mockUseAdminOAuthClients.mockReturnValue({
      data: clientsResponse,
      isLoading: false,
      isFetching: true,
      isPlaceholderData: true,
      error: null,
      refetch: mockRefetch,
    });

    render(<AdminOAuthClientsPage />);

    expect(screen.getByRole("status")).toHaveTextContent("Updating results");
    expect(screen.getByText("Showing 1-1 of 1 clients")).toBeInTheDocument();
    expect(screen.getByText("Page 1 of 1")).toBeInTheDocument();
    expect(
      screen.getByRole("switch", {
        name: "Toggle broker capability for Aevatar (dcr-aevatar)",
      }),
    ).toBeDisabled();

    const overlay = screen.getByTestId("oauth-clients-refetch-overlay");
    expect(overlay).toBeInTheDocument();
    const overlayWrapper = overlay.parentElement;
    expect(overlayWrapper).not.toBeNull();
    expect(overlayWrapper).toHaveAttribute("aria-busy", "true");
    expect(overlayWrapper).toHaveClass("opacity-60", "pointer-events-none");
  });

  it("hides the refetch overlay when the current results match the query", () => {
    render(<AdminOAuthClientsPage />);

    expect(
      screen.queryByTestId("oauth-clients-refetch-overlay"),
    ).not.toBeInTheDocument();
  });

  it("warns and retries when a cached-data refresh fails", async () => {
    const user = userEvent.setup();
    mockUseAdminOAuthClients.mockReturnValue({
      data: clientsResponse,
      isLoading: false,
      isFetching: false,
      isPlaceholderData: false,
      error: new Error("refresh failed"),
      refetch: mockRefetch,
    });

    render(<AdminOAuthClientsPage />);

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Results may be out of date because the refresh failed.",
    );
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(mockRefetch).toHaveBeenCalledOnce();
  });

  it("confirms and disables effective broker capability by removing the legacy scope trigger", async () => {
    const user = userEvent.setup();
    render(<AdminOAuthClientsPage />);

    await user.click(
      screen.getByRole("switch", {
        name: "Toggle broker capability for Aevatar (dcr-aevatar)",
      }),
    );

    expect(
      screen.getByText(/removes the legacy broker-binding scope/i),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Disable capability" }),
    );

    expect(mockUpdateClient).toHaveBeenCalledWith({
      clientId: "dcr-aevatar",
      data: {
        broker_capability_enabled: false,
        allowed_scopes: ["openid"],
      },
    });
  });

  it("shows every write control and the broker policy card to admins", () => {
    render(<AdminOAuthClientsPage />);

    expect(screen.getByText("Broker Rollout Policy")).toBeInTheDocument();
    expect(
      screen.getByRole("switch", {
        name: "Toggle broker capability for Aevatar (dcr-aevatar)",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", {
        name: "Toggle active status for Aevatar (dcr-aevatar)",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", {
        name: "Toggle Require sender constraint",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", {
        name: "Toggle Require admin broker capability",
      }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reset" })).toBeInTheDocument();
    expect(screen.getByText("Overridden")).toBeInTheDocument();
    expect(screen.getByText("Env default")).toBeInTheDocument();
    expect(screen.getByText("Broker scope")).toBeInTheDocument();
    expect(screen.getByText("Broker enabled")).toBeInTheDocument();
    expect(screen.getByText("Active")).toBeInTheDocument();
    expect(screen.getAllByText("Enabled").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Disabled").length).toBeGreaterThan(0);
    expect(mockUseBrokerSettings).toHaveBeenCalledWith(true);
  });

  it("shows operators a read-only client list without broker policy controls", () => {
    useAuthStore.setState({ user: operatorUser() });

    render(<AdminOAuthClientsPage />);

    expect(screen.getByText("Aevatar")).toBeInTheDocument();
    expect(screen.getByText("Broker scope")).toBeInTheDocument();
    expect(screen.getByText("Broker enabled")).toBeInTheDocument();
    expect(screen.getByText("Active")).toBeInTheDocument();
    expect(screen.queryByText("Broker Rollout Policy")).not.toBeInTheDocument();
    expect(screen.queryAllByRole("switch")).toHaveLength(0);
    expect(
      screen.queryByRole("button", { name: "Reset" }),
    ).not.toBeInTheDocument();
    expect(mockUseBrokerSettings).toHaveBeenCalledWith(false);
  });

  it("resets an overridden broker setting to the env default", async () => {
    const user = userEvent.setup();
    render(<AdminOAuthClientsPage />);

    await user.click(screen.getByRole("button", { name: "Reset" }));
    await user.click(
      screen.getByRole("button", { name: "Reset to env default" }),
    );

    expect(mockUpdateSettings).toHaveBeenCalledWith({
      broker_require_sender_constraint: null,
    });
  });

  it("summarizes long scope lists and reveals the complete list", async () => {
    const user = userEvent.setup();
    mockUseAdminOAuthClients.mockReturnValue({
      data: {
        ...clientsResponse,
        clients: [
          {
            ...clientsResponse.clients[0],
            allowed_scopes:
              "openid profile email offline_access roles groups proxy",
          },
        ],
      },
      isLoading: false,
      isFetching: false,
      isPlaceholderData: false,
      error: null,
      refetch: mockRefetch,
    });

    render(<AdminOAuthClientsPage />);

    expect(screen.getByText("openid")).toBeInTheDocument();
    expect(screen.getByText("profile")).toBeInTheDocument();
    expect(screen.getByText("email")).toBeInTheDocument();
    expect(screen.queryByText("offline_access")).not.toBeInTheDocument();

    await user.click(
      screen.getByRole("button", {
        name: "View all 7 allowed scopes: openid, profile, email, offline_access, roles, groups, proxy",
      }),
    );

    expect(screen.getByText("offline_access")).toBeInTheDocument();
    expect(screen.getByText("roles")).toBeInTheDocument();
    expect(screen.getByText("groups")).toBeInTheDocument();
    expect(screen.getByText("proxy")).toBeInTheDocument();
  });

  it("lets keyboard and touch users reveal a single long scope", () => {
    render(<AdminOAuthClientsPage />);

    const scopeDisclosure = screen.getByRole("button", {
      name: /^View all 2 allowed scopes:/,
    });
    scopeDisclosure.focus();
    expect(scopeDisclosure).toHaveFocus();
    fireEvent.click(scopeDisclosure);

    expect(
      screen.getByText("Allowed scopes", { selector: "p" }),
    ).toBeInTheDocument();
    expect(screen.getAllByText("urn:nyxid:scope:broker_binding")).toHaveLength(
      2,
    );
  });
});

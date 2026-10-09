import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { ToolOffering } from "@/schemas/tools";
import { ToolsPage } from "./tools";
import { AdminToolsPage } from "./admin-tools";
const mocks = vi.hoisted(() => ({
  post: vi.fn(),
  admin: true,
  tools: [] as ToolOffering[],
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: { children: ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));
vi.mock("@/lib/api-client", () => ({
  api: { post: mocks.post, get: vi.fn() },
}));
vi.mock("@/hooks/use-tools", () => ({
  useTools: () => ({ data: mocks.tools, isLoading: false }),
  useToolTopics: () => ({
    data: [{ slug: "web-search", label: "Web Search" }],
  }),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (
    selector: (state: { user: { is_admin: boolean } }) => unknown,
  ) => selector({ user: { is_admin: mocks.admin } }),
}));
vi.mock("@/hooks/use-api-keys", () => ({ useApiKeys: () => ({ data: [] }) }));
vi.mock("@/hooks/use-services", () => ({
  useServices: () => ({
    data: [
      {
        id: "source-1",
        name: "Firecrawl",
        slug: "api-firecrawl",
        service_type: "http",
        offering_kind: "ai_service",
        auth_method: "bearer",
      },
      {
        id: "tool-1",
        name: "Search",
        slug: "search",
        offering_kind: "tool",
        supplier: "Acme",
        auth_method: "bearer",
        credential_configured: true,
        openapi_spec_url: "https://example.com/spec",
        topics: [],
      },
    ],
    isLoading: false,
  }),
}));
vi.mock("@/hooks/use-endpoints", () => ({
  useEndpoints: () => ({
    data: [
      {
        id: "op-1",
        name: "search",
        method: "GET",
        path: "/search",
        publication: "draft",
        is_active: false,
      },
    ],
  }),
}));
function tool(
  id: string,
  price: ToolOffering["pricing"]["platform"],
  byok: boolean,
): ToolOffering {
  return {
    id,
    slug: id,
    name: id,
    description: "Search public data",
    supplier: "Acme",
    topics: ["web-search"],
    homepage_url: null,
    provider_label: "Acme",
    offering_kind: "tool",
    access: { platform: true, byok },
    pricing: { platform: price, byok: null },
    limits: { rate_limit_per_second: 5, burst: 10 },
    credential_configured: true,
    operations: [
      {
        name: "search",
        description: "Search",
        method: "GET",
        path: "/search",
        data_scope: "public",
        cost_class: "metered",
        execution: "http_operation",
        risk: "read",
      },
    ],
  };
}
function mount(element: ReactNode) {
  return render(
    <QueryClientProvider
      client={
        new QueryClient({
          defaultOptions: {
            queries: { retry: false },
            mutations: { retry: false },
          },
        })
      }
    >
      {element}
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  mocks.admin = true;
  mocks.post.mockReset();
  mocks.post.mockResolvedValue({});
  mocks.tools = [
    tool("Free search", "free", false),
    tool(
      "Paid search",
      { metric: "requests", credits_per_unit: "0.012" },
      true,
    ),
  ];
});
it("groups suppliers and renders lane prices, topics and BYOK actions", async () => {
  mount(<ToolsPage />);
  expect(screen.getAllByRole("heading", { name: "Acme" })).toHaveLength(1);
  expect(screen.getByText("Free")).toBeInTheDocument();
  expect(screen.getByText("0.012 credits / request")).toBeInTheDocument();
  expect(screen.getAllByRole("link", { name: "Use my own key" })).toHaveLength(
    1,
  );
  await userEvent.type(
    screen.getByRole("textbox", { name: "Search tools" }),
    "Paid",
  );
  expect(
    screen.queryByRole("heading", { name: "Free search" }),
  ).not.toBeInTheDocument();
});

it("renders the effective operation price range and each operation price", () => {
  const operation = (name: string) => ({
    name,
    description: name,
    method: "GET",
    path: `/${name}`,
    data_scope: "public" as const,
    cost_class: "metered" as const,
    execution: "http_operation" as const,
    risk: "read" as const,
  });
  mocks.tools = [
    {
      ...tool(
        "Ranged search",
        {
          metric: "requests",
          credits_per_unit: "0.1",
          operations: [
            {
              operation: "search",
              label: "search",
              credits_per_unit: "0.25",
              sync_status: "synced",
            },
            {
              operation: "lookup",
              label: "lookup",
              credits_per_unit: "0.05",
              sync_status: "synced",
            },
            {
              operation: "export",
              label: "export",
              credits_per_unit: "9",
              sync_status: "pending",
            },
          ],
        },
        false,
      ),
      operations: ["search", "lookup", "export"].map(operation),
    },
  ];
  mount(<ToolsPage />);
  expect(
    screen.getByText("From 0.05 to 0.25 credits per request"),
  ).toBeInTheDocument();
  // A pending price is not charged yet, so its operation shows the base price.
  for (const price of ["0.25", "0.05", "0.1"])
    expect(screen.getByText(`${price} credits / request`)).toBeInTheDocument();
  expect(screen.queryByText("9 credits / request")).not.toBeInTheDocument();
});
it("publishes through the publication route from the Tools tab", async () => {
  mount(<AdminToolsPage />);
  await userEvent.click(screen.getByRole("tab", { name: "Tools" }));
  await userEvent.click(screen.getByRole("button", { name: "Publish" }));
  await waitFor(() =>
    expect(mocks.post).toHaveBeenCalledWith(
      "/services/tool-1/endpoints/op-1/publication",
      { state: "published" },
    ),
  );
});
it("keeps Phase 1 management admin-only", () => {
  mocks.admin = false;
  mount(<AdminToolsPage />);
  expect(
    screen.queryByRole("button", { name: "Add tool" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("tab", { name: "Imports" }),
  ).not.toBeInTheDocument();
});

it("creates a twin with the selected source and no transport or credential fields", async () => {
  mocks.post.mockResolvedValue({ id: "created-tool", name: "Firecrawl tools" });
  mount(<AdminToolsPage />);
  await userEvent.click(screen.getByRole("button", { name: "Add tool" }));
  expect(screen.queryByLabelText("Base URL")).not.toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("combobox", { name: "Source service" }),
  );
  await userEvent.click(
    screen.getByRole("option", { name: "Firecrawl (api-firecrawl)" }),
  );
  await userEvent.type(screen.getByLabelText("name"), "Firecrawl tools");
  await userEvent.type(screen.getByLabelText("slug"), "tools-firecrawl");
  await userEvent.click(screen.getByRole("button", { name: "Create tool" }));
  await waitFor(() =>
    expect(mocks.post).toHaveBeenCalledWith(
      "/services",
      expect.objectContaining({
        twin_of_service_id: "source-1",
        slug: "tools-firecrawl",
        offering_kind: "tool",
      }),
    ),
  );
  const body = mocks.post.mock.calls[0]![1];
  expect(body).not.toHaveProperty("credential");
  expect(body).not.toHaveProperty("base_url");
  expect(body).not.toHaveProperty("provider_config_id");
  expect(screen.getByText(/draft operations/)).toBeInTheDocument();
});
it("offers a new-service path without an Imports tab", async () => {
  mount(<AdminToolsPage />);
  expect(
    screen.queryByRole("tab", { name: "Imports" }),
  ).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Add tool" }));
  await userEvent.click(
    screen.getByRole("combobox", { name: "Creation method" }),
  );
  await userEvent.click(screen.getByRole("option", { name: "New service" }));
  expect(screen.getByLabelText("Base URL")).toBeEnabled();
  expect(screen.getByLabelText("OpenAPI spec URL")).toBeInTheDocument();
});

it("renders disabled limits and labels writes as changes to data", async () => {
  mocks.tools = [tool("Write tool", "free", false)];
  mocks.tools[0]!.limits = null;
  mocks.tools[0]!.operations[0]!.risk = "write";
  mount(<ToolsPage />);
  expect(screen.getByText("No per-user rate limit")).toBeInTheDocument();
  await userEvent.click(screen.getByText("1 operations"));
  expect(screen.getByText("Changes data")).toBeInTheDocument();
  expect(screen.queryByText("Destructive")).not.toBeInTheDocument();
});

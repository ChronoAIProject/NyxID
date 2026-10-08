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
  useToolEditorAuthority: () => ({
    data: { read: true, write: true, admin: mocks.admin },
  }),
}));
vi.mock("@/hooks/use-api-keys", () => ({ useApiKeys: () => ({ data: [] }) }));
vi.mock("@/hooks/use-services", () => ({
  useServices: () => ({
    data: [
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
it.each([true, false])("gates transport fields for admin=%s", async (admin) => {
  mocks.admin = admin;
  mount(<AdminToolsPage />);
  await userEvent.click(screen.getByRole("button", { name: "Add tool" }));
  const input = screen.getByLabelText("Base URL");
  if (admin) expect(input).toBeEnabled();
  else expect(input).toBeDisabled();
  expect(screen.getByLabelText("name")).toBeEnabled();
});

it.each([true, false])(
  "gates offering conversion for admin=%s",
  async (admin) => {
    mocks.admin = admin;
    mount(<AdminToolsPage />);
    await userEvent.click(screen.getByRole("tab", { name: "Tools" }));
    await userEvent.click(screen.getByText("Edit metadata"));
    const field = screen.getByLabelText("Offering kind");
    if (admin) expect(field).toBeEnabled();
    else expect(field).toBeDisabled();
  },
);

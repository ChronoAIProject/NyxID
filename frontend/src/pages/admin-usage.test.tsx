import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { usageFixture } from "@/test/admin-usage-fixture";
import { useAdminUsage } from "@/hooks/use-admin-usage";
import { EMPTY_FILTERS } from "@/lib/usage-analytics";
import { AdminUsageList as AdminUsagePage } from "@/components/billing-analytics/usage-list";
const mocks = vi.hoisted(() => ({
  navigate: vi.fn(),
  refetch: vi.fn(),
  search: {} as Record<string, unknown>,
}));
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mocks.navigate,
  useSearch: () => mocks.search,
}));
vi.mock("@/hooks/use-admin-usage", () => ({ useAdminUsage: vi.fn() }));
function renderPage() {
  return render(
    <TooltipProvider>
      <AdminUsagePage filters={EMPTY_FILTERS} />
    </TooltipProvider>,
  );
}
function setUsage(overrides: Record<string, unknown> = {}) {
  vi.mocked(useAdminUsage).mockReturnValue({
    data: usageFixture(),
    isPending: false,
    isError: false,
    isFetching: false,
    refetch: mocks.refetch,
    ...overrides,
  } as unknown as ReturnType<typeof useAdminUsage>);
}
beforeEach(() => {
  vi.clearAllMocks();
  mocks.search = {};
  setUsage();
});
it("renders totals, token classes, costs, user names and org attribution without bare IDs", () => {
  renderPage();
  expect(
    screen.getByRole("heading", { name: "By service" }),
  ).toBeInTheDocument();
  expect(screen.getAllByText("Alice").length).toBeGreaterThan(0);
  expect(screen.getAllByText("alice@example.test").length).toBeGreaterThan(0);
  expect(screen.getAllByText("Research team").length).toBeGreaterThan(0);
  expect(
    screen.queryByText(usageFixture().ranking[0]!.user.id),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole("heading", { name: "Platform key vs own key" }),
  ).toBeInTheDocument();
});
it("writes ranking and pagination changes into the list URL", async () => {
  const user = userEvent.setup();
  renderPage();
  await user.click(screen.getByRole("combobox", { name: "Ranking sort" }));
  await user.click(screen.getByRole("option", { name: "Gross cost" }));
  expect(mocks.navigate).toHaveBeenLastCalledWith({
    to: "/admin/usage",
    search: expect.objectContaining({ tab: "list", sort: "cost" }),
  });
  await user.click(screen.getByRole("button", { name: "Next page" }));
  expect(mocks.navigate).toHaveBeenLastCalledWith({
    to: "/admin/usage",
    search: expect.objectContaining({ tab: "list", page: 2 }),
  });
});
it("expands user services against the exact response window", async () => {
  renderPage();
  await userEvent.click(
    screen.getAllByRole("button", { name: "Services" })[0]!,
  );
  expect(screen.getAllByText("Services for Alice").length).toBeGreaterThan(0);
  expect(useAdminUsage).toHaveBeenCalledWith(
    expect.objectContaining({
      user: usageFixture().ranking[0]!.user.id,
      service: undefined,
      period: "custom",
      from: "2026-09-18T00:00:00Z",
      to: "2026-09-19T00:00:00Z",
    }),
  );
});
it("shows the billing prerequisite in the empty state", () => {
  const data = usageFixture();
  data.totals.events = 0;
  data.ranking = [];
  setUsage({ data });
  renderPage();
  expect(screen.getByText("No usage in this window.")).toBeInTheDocument();
  expect(screen.getByText(/usage metering enabled/)).toBeInTheDocument();
});
it("renders loading, retryable errors and invalid ranges distinctly", async () => {
  setUsage({ isPending: true, data: undefined });
  const view = renderPage();
  expect(screen.getByLabelText("Loading usage")).toBeInTheDocument();
  setUsage({
    isError: true,
    error: new Error("Usage query timed out"),
    data: undefined,
  });
  view.rerender(
    <TooltipProvider>
      <AdminUsagePage filters={EMPTY_FILTERS} />
    </TooltipProvider>,
  );
  expect(screen.getByText("Usage query timed out")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(mocks.refetch).toHaveBeenCalledOnce();

  view.rerender(
    <TooltipProvider>
      <AdminUsagePage
        filters={{
          ...EMPTY_FILTERS,
          period: "custom",
          from: "2026-01-01T00:00:00Z",
          to: "2026-03-01T00:00:00Z",
        }}
      />
    </TooltipProvider>,
  );
  expect(screen.getByRole("alert")).toHaveTextContent("31 days");
});
it("keeps a future metric visible and exposes partial historical pricing", () => {
  const data = usageFixture();
  data.totals.quantities.future_units = 42;
  data.totals.unknown_cost_events = 3;
  setUsage({ data });
  renderPage();

  expect(screen.getByText(/Costs are partial: 3/)).toBeInTheDocument();
});

it.each([
  ["2026-09-17T23:59:00Z", "Backfilling history · rollups complete through"],
  ["2026-09-18T00:00:00Z", "Live · includes 417 unfolded rows"],
])("surfaces freshness watermark %s", (rolled_up_through, text) => {
  const data = usageFixture();
  data.freshness = { rolled_up_through, tail_rows: 417, validated: true };
  setUsage({ data });
  renderPage();
  expect(
    screen.getByText(
      (_, element) =>
        element?.tagName === "P" &&
        Boolean(element.textContent?.includes(text)),
    ),
  ).toBeInTheDocument();
});
it("labels totals that could not be validated during a fold", () => {
  const data = usageFixture();
  data.freshness = {
    rolled_up_through: data.window.from,
    tail_rows: 417,
    validated: false,
  };
  setUsage({ data });
  renderPage();
  expect(screen.getByText(/Updating totals/)).toBeInTheDocument();
});

it("applies the shared dashboard filters and resets list pagination when they change", async () => {
  mocks.search = { page: 4 };
  const page = renderPage();
  const filters = {
    ...EMPTY_FILTERS,
    services: ["llm-example"],
    actors: [usageFixture().ranking[0]!.user.id],
    owners: ["33333333-3333-4333-8333-333333333333"],
  };
  page.rerender(
    <TooltipProvider>
      <AdminUsagePage filters={filters} />
    </TooltipProvider>,
  );
  expect(useAdminUsage).toHaveBeenLastCalledWith(
    expect.objectContaining({
      ...filters,
      from: undefined,
      to: undefined,
      page: 4,
    }),
  );
  await waitFor(() =>
    expect(mocks.navigate).toHaveBeenLastCalledWith(
      expect.objectContaining({
        to: "/admin/usage",
        replace: true,
        search: expect.objectContaining({ tab: "list", page: 1 }),
      }),
    ),
  );
});

import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import { FreeGrantsReport } from "./free-grants-report";
import { freeGrantParams } from "@/hooks/use-free-grants-report";
import { EMPTY_FILTERS } from "@/lib/usage-analytics";
import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
const { get } = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  api: { get },
}));
const window = { from: "2026-10-01T00:00:00Z", to: "2026-10-02T00:00:00Z" };
const response = (page = 1, servicePage = 1) => ({
  window,
  generated_at: window.to,
  issued: "123.000000000001",
  used: "200.000000000002",
  grants: 70,
  total_accounts: 51,
  page,
  per_page: 50,
  total_services: 51,
  service_page: servicePage,
  interval: "day",
  trend: [
    {
      start: window.from,
      end: window.to,
      issued: "123.000000000001",
      used: "200.000000000002",
    },
  ],
  services: [
    {
      service_slug: servicePage === 1 ? "nyxbot" : null,
      used: "0.000000000001",
    },
  ],
  accounts: [
    {
      owner_id: `account-${page}`,
      label: page === 1 ? "Ada" : "Research",
      account_kind: page === 1 ? "person" : "org",
      issued: "1.000000000001",
      used: "5.000000000001",
      grants: 1,
    },
  ],
});
function mount() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <FreeGrantsReport filters={EMPTY_FILTERS} />
    </QueryClientProvider>,
  );
  return client;
}
async function expand() {
  await userEvent.click(
    await screen.findByRole("button", { name: "View breakdown" }),
  );
}
beforeEach(() => {
  useAuthStore.setState({ user: { id: "admin-a" } as User });
  get.mockReset();
  get.mockResolvedValue(response());
});
it("shows complete granted and used totals before the collapsed breakdown, without a utilization rate", async () => {
  mount();
  expect(await screen.findByTitle("123.000000000001")).toHaveTextContent(
    "123 credits",
  );
  expect(screen.getByTitle("200.000000000002")).toHaveTextContent(
    "200 credits",
  );
  expect(
    screen.getByText(/Usage can include older grants and exceed/),
  ).toBeVisible();
  expect(screen.getByText("Ada")).not.toBeVisible();
  expect(
    screen.getByRole("button", { name: "View breakdown" }),
  ).toHaveAttribute("aria-expanded", "false");
  expect(
    screen.getByRole("img", { name: /Daily grant credits/ }),
  ).toBeVisible();
  expect(screen.queryByText(/%/)).not.toBeInTheDocument();
  await expand();
  expect(screen.getByText(/70 grants · 51 billing accounts/)).toBeVisible();
  expect(screen.getByText("Personal account · 1 grant")).toBeVisible();
  expect(screen.getByTitle("1.000000000001")).toHaveTextContent("1 credit");
  expect(screen.getByTitle("5.000000000001")).toHaveTextContent("5 credits");
  const params = freeGrantParams({
    ...EMPTY_FILTERS,
    owners: ["z", "a"],
    actors: ["actor"],
    services: ["service"],
  });
  expect(params.get("owners")).toBe("a,z");
  expect([...params.keys()]).toEqual(["period", "owners"]);
});
it("exposes exact accessible daily chart data and usage-only services without inventing issuance", async () => {
  mount();
  await screen.findByRole("img", { name: /Daily grant credits/ });
  await userEvent.click(screen.getByText("View chart data"));
  const table = screen.getByRole("table", {
    name: /Exact daily grant credits/,
  });
  expect(within(table).getByText("123.000000000001")).toBeVisible();
  expect(within(table).getByText("200.000000000002")).toBeVisible();
  await expand();
  await userEvent.click(screen.getByRole("tab", { name: "Service" }));
  expect(
    screen.getByText(/Shared grants have no per-service granted amount/),
  ).toBeVisible();
  const service = screen.getByRole("tabpanel", { name: "Service" });
  expect(within(service).getByText("nyxbot")).toBeVisible();
  expect(within(service).getByTitle("0.000000000001")).toHaveTextContent(
    "<0.000001 credits used",
  );
  expect(within(service).queryByText("Granted")).not.toBeInTheDocument();
  expect(screen.getAllByTitle("123.000000000001")[0]).toBeVisible();
});
it("uses singular credit, grant, and billing account labels", async () => {
  const data = response();
  get.mockResolvedValue({
    ...data,
    issued: "1",
    used: "0",
    grants: 1,
    total_accounts: 1,
    accounts: [{ ...data.accounts[0], issued: "1" }],
  });
  mount();
  await screen.findByLabelText("Complete grant credit totals");
  expect(
    within(screen.getByLabelText("Complete grant credit totals")).getByTitle(
      "1",
    ),
  ).toHaveTextContent("1 credit");
  await expand();
  expect(screen.getByText(/1 grant · 1 billing account with/)).toBeVisible();
  expect(screen.getByText("Personal account · 1 grant")).toBeVisible();
});
it("preserves expansion and totals during account paging and independently pages services in the pinned window", async () => {
  let resolve: (v: unknown) => void = () => {};
  get.mockImplementation((path: string) => {
    const p = new URLSearchParams(path.split("?")[1]);
    if (p.get("page") === "2" && p.get("service_page") === "1")
      return new Promise((r) => {
        resolve = r;
      });
    return Promise.resolve(
      response(Number(p.get("page")), Number(p.get("service_page"))),
    );
  });
  mount();
  await expand();
  await userEvent.click(screen.getByRole("button", { name: "Next accounts" }));
  expect(screen.getByText("Ada")).toBeVisible();
  expect(screen.getByRole("button", { name: "Next accounts" })).toBeDisabled();
  const params = new URLSearchParams(get.mock.calls.at(-1)![0].split("?")[1]);
  expect(params.get("from")).toBe(window.from);
  expect(params.get("to")).toBe(window.to);
  expect(params.has("period")).toBe(false);
  await act(async () => resolve(response(2)));
  expect(await screen.findByText("Research")).toBeVisible();
  expect(screen.getByText("Page 2 of 2")).toBeVisible();
  await userEvent.click(screen.getByRole("tab", { name: "Service" }));
  expect(screen.getByText("Page 1 of 2")).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "Next services" }));
  expect(
    await screen.findByText("Historical usage · service unavailable"),
  ).toBeVisible();
  expect(get.mock.calls.at(-1)![0]).toContain("page=2&service_page=2");
  expect(screen.getByTitle("123.000000000001")).toBeVisible();
  expect(screen.getByTitle("200.000000000002")).toBeVisible();
  await userEvent.click(screen.getByRole("tab", { name: "Account" }));
  expect(screen.getByText("Research")).toBeVisible();
});
it("labels stale refreshes and clears metrics, chart, and details after lost authorization", async () => {
  const client = mount();
  await expand();
  get.mockRejectedValue(new Error("offline"));
  await act(() => client.invalidateQueries());
  expect(
    await screen.findByText(
      "Showing previously loaded grant totals. Refresh failed.",
    ),
  ).toBeVisible();
  expect(screen.getByText("Ada")).toBeVisible();
  get.mockRejectedValue(
    new ApiError(403, {
      error: "forbidden",
      error_code: 1003,
      message: "Forbidden",
    }),
  );
  await act(() => client.invalidateQueries());
  await waitFor(() =>
    expect(screen.queryByText("Ada")).not.toBeInTheDocument(),
  );
  expect(screen.queryByTitle("123.000000000001")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("img", { name: /Daily grant credits/ }),
  ).not.toBeInTheDocument();
});
it("does not carry previous admin totals into a different signed-in identity", async () => {
  mount();
  await screen.findByTitle("123.000000000001");
  get.mockImplementation(() => new Promise(() => {}));
  await act(async () =>
    useAuthStore.setState({ user: { id: "admin-b" } as User }),
  );
  expect(screen.queryByTitle("123.000000000001")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "View breakdown" }),
  ).not.toBeInTheDocument();
});
it("renders the empty population accurately", async () => {
  get.mockResolvedValue({
    ...response(),
    issued: "0",
    used: "0",
    grants: 0,
    total_accounts: 0,
    total_services: 0,
    accounts: [],
    services: [],
    trend: [{ start: window.from, end: window.to, issued: "0", used: "0" }],
  });
  mount();
  await expand();
  expect(
    screen.getByText("No grant issuance or usage in this window."),
  ).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Next accounts" }),
  ).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("tab", { name: "Service" }));
  expect(
    screen.getByText("No grant-funded service usage on this page."),
  ).toBeVisible();
});
it("starts both pages at one with the current relative window on explicit Refresh", async () => {
  get.mockImplementation((path: string) => {
    const p = new URLSearchParams(path.split("?")[1]);
    return Promise.resolve(
      response(Number(p.get("page")), Number(p.get("service_page"))),
    );
  });
  const client = new QueryClient();
  const ui = (refreshKey: number) => (
    <QueryClientProvider client={client}>
      <FreeGrantsReport filters={EMPTY_FILTERS} refreshKey={refreshKey} />
    </QueryClientProvider>
  );
  const view = render(ui(0));
  await expand();
  await userEvent.click(screen.getByRole("button", { name: "Next accounts" }));
  await screen.findByText("Research");
  view.rerender(ui(1));
  await waitFor(() =>
    expect(get.mock.calls.at(-1)![0]).toContain(
      "period=7d&page=1&service_page=1",
    ),
  );
  await expand();
  expect(screen.getByText("Ada")).toBeVisible();
});

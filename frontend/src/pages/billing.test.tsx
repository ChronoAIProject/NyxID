import { withAccountPanelSearch } from "@/lib/assistant/account-panel-search";
import { act, render, screen, within, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useSyncExternalStore } from "react";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { billingSearchSchema } from "@/schemas/billing";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ApiError } from "@/lib/api-client";
import {
  billingCatalog,
  billingRow as row,
  billingUsage as usage,
  billingWallet,
  billingAgentUsage,
} from "@/test/billing-fixture";
import { BillingPage } from "./billing";

const mocks = vi.hoisted(() => ({
  wallet: vi.fn(),
  usage: vi.fn(),
  history: vi.fn(),
  grants: vi.fn(),
  allowances: vi.fn(),
  catalog: vi.fn(),
  activity: vi.fn(),
  provision: vi.fn(),
  topup: vi.fn(),
  receipt: vi.fn(),
  openExternal: vi.fn(),
}));
vi.mock("@/hooks/use-billing", () => ({
  useBillingWallet: mocks.wallet,
  useBillingUsage: mocks.usage,
  useTopUpHistory: mocks.history,
  useProvisionBillingWallet: () => ({ mutateAsync: mocks.provision }),
  useTopUpBilling: () => ({ mutateAsync: mocks.topup }),
  openInvoiceReceipt: mocks.receipt,
}));
vi.mock("@/hooks/use-billing-credits", () => ({
  useActiveCreditGrants: mocks.grants,
  useCurrentAllowances: mocks.allowances,
}));
vi.mock("@/hooks/use-keys", () => ({ useCatalog: mocks.catalog }));
vi.mock("@/hooks/use-api-keys", () => ({ useApiKeysUsage: mocks.activity }));
vi.mock("@/lib/navigation", () => ({ openExternal: mocks.openExternal }));
const query = (data: unknown, error: unknown = null) => ({
  data,
  error,
  isLoading: false,
  isSuccess: !error,
  isFetching: false,
  isError: Boolean(error),
  refetch: vi.fn(),
});
const PAGE_ROUTES = ["/billing", "/assistant"] as const;
let pageRoute: (typeof PAGE_ROUTES)[number] = "/billing";
async function renderPage(url = "/billing?tab=usage", exploreGroups = true) {
  if (pageRoute === "/assistant") {
    const old = new URL(url, "https://nyxid.test");
    const params = new URLSearchParams({ panel: "billing", c: "chat", mock: "1" });
    for (const [key, value] of old.searchParams) params.set(`panel${key.charAt(0).toUpperCase()}${key.slice(1)}`, value);
    url = `/assistant?${params}`;
  }
  const root = createRootRoute();
  const route = createRoute({
    getParentRoute: () => root,
    path: pageRoute,
    validateSearch: pageRoute === "/billing" ? billingSearchSchema.parse : withAccountPanelSearch((search) => ({ c: search.c, mock: search.mock })),
    component: () => pageRoute === "/billing"
      ? <BillingPage />
      : <BillingPage presentation="panel" />,
  });
  const history = createMemoryHistory({ initialEntries: [url] });
  const router = createRouter({
    routeTree: root.addChildren([route]),
    history,
  });
  render(
    <TooltipProvider>
      <RouterProvider router={router} />
    </TooltipProvider>,
  );
  await act(() => router.load());
  await screen.findByRole("tab", { name: "Billing", hidden: true });
  if (pageRoute === "/assistant") expect(router.state.location.search).toMatchObject({ c: "chat", mock: 1 });
  expect(history.location.pathname).toBe(pageRoute);
  if (exploreGroups) {
    const disclosure = screen.queryByText("Explore by service, model or agent");
    if (disclosure) await userEvent.click(disclosure);
  }
  return { history, router };
}
async function select(label: string, option: string) {
  await userEvent.click(screen.getByRole("combobox", { name: label }));
  await userEvent.click(screen.getByRole("option", { name: option }));
}
function serviceFilter() {
  return screen.getByRole("button", { name: "Filter services" });
}
const picker = () => within(screen.getByRole("dialog"));
/** An option row; its name is the service followed by its slug line. */
const option = (name: string) =>
  picker().getByRole("checkbox", { name: new RegExp(`^${name}`) });
const servicesChip = (name?: string) =>
  screen.queryAllByRole("button", {
    name: name ? `Edit service filter: ${name}` : /^Edit service filter:/,
  })[0] ?? null;
async function selectServices(names: string[]) {
  for (const name of names) await userEvent.click(option(name));
  await userEvent.click(picker().getByRole("button", { name: "Done" }));
}
function servicesParam(history: { location: { search: string } }) {
  const raw = new URLSearchParams(history.location.search).get(pageRoute === "/assistant" ? "panelServices" : "services");
  return raw === null ? undefined : (JSON.parse(raw) as string[]);
}
function twoServiceRows() {
  return [
    row(),
    row({
      service_slug: "free-service",
      quantity: 10,
      metric: "requests",
      estimated_credits_micros: 0,
      grant_credits_micros: 0,
      billable: false,
    }),
  ];
}
beforeEach(() => {
  vi.resetAllMocks();
  mocks.wallet.mockReturnValue(query(billingWallet()));
  mocks.usage.mockReturnValue(query(usage()));
  mocks.history.mockReturnValue(query({ topups: [], total: 0 }));
  mocks.grants.mockReturnValue(query({ grants: [] }));
  mocks.allowances.mockReturnValue(query({ allowances: [] }));
  mocks.catalog.mockReturnValue(query(billingCatalog));
  mocks.activity.mockReturnValue(query([]));
});
describe.each(PAGE_ROUTES)("BillingPage at %s", (route) => {
  beforeEach(() => { pageRoute = route; });
  it("opens on visual quantities and funding with the grouped list collapsed", async () => {
    await renderPage("/billing?tab=usage", false);
    expect(screen.getByText("Usage overview", { exact: true })).toBeVisible();
    expect(
      screen.getByRole("region", { name: "Funding composition" }),
    ).toBeVisible();
    expect(
      screen.getByRole("combobox", { name: "Compare quantities by" }),
    ).toHaveTextContent("Service");
    expect(
      document.querySelector(".usage-group-disclosure"),
    ).not.toHaveAttribute("open");
    expect(
      document.querySelector(".expandable-service > summary"),
    ).not.toBeVisible();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByText("Explore by service, model or agent"),
    );
    expect(
      document.querySelector(".expandable-service > summary"),
    ).toBeVisible();
  });
  it("defaults to Billing and preserves the wallet and top-up history actions", async () => {
    await renderPage("/billing");
    expect(screen.getByRole("tab", { name: "Billing" })).toHaveAttribute(
      "data-state",
      "active",
    );
    expect(screen.getByText("95 credits")).toBeVisible();
    expect(screen.queryByText("Usage overview")).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "View breakdown" }),
    );
    expect(screen.getByText("100 credits")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Add credits" }));
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(screen.getAllByText("Top-up history")).toHaveLength(1);
  });
  it("preserves exact fractional funding and independent allowance quantities in disclosures", async () => {
    mocks.usage.mockReturnValue(
      query(
        usage([
          row({
            wallet_credits_micros: 240,
            grant_credits_micros: 1000,
            allowance_credits_micros: 1200,
            allowance_quantity: 1200,
          }),
        ]),
      ),
    );
    await renderPage();
    expect(
      screen.getAllByText("0.00244", { exact: false }).length,
    ).toBeGreaterThan(0);
    await userEvent.click(screen.getByText("All metrics & funding"));
    const funding = screen.getByText("Funding breakdown").parentElement!;
    expect(within(funding).getByText("0.001 credits")).toBeVisible();
    expect(within(funding).getByText("0.0012 credits")).toBeVisible();
    expect(within(funding).getByText("0.00024 credits")).toBeVisible();
    expect(within(funding).getByText("1,200")).toBeVisible();
    await userEvent.click(
      screen.getByText("Example LLM", {
        selector: ".expandable-name > strong",
      }),
    );
    expect(
      screen.queryByRole("tab", { name: /^Records/ }),
    ).not.toBeInTheDocument();
    const detail = document.querySelector(".detailed-usage")!;
    expect(
      within(detail as HTMLElement).getByRole("region", {
        name: "Funding composition",
      }),
    ).toHaveTextContent("0.00024 credits");
    expect(detail.querySelector(".usage-quantity-chart")).toBeVisible();
  });
  it("offers only used services and filters every summary, detail, and funding value", async () => {
    mocks.usage.mockReturnValue(query(usage(twoServiceRows())));
    const { history } = await renderPage();
    expect(
      screen.getByRole("combobox", { name: "Time range" }),
    ).toHaveTextContent("Last 30 days");
    expect(servicesChip()).toBeNull();
    await userEvent.click(serviceFilter());
    expect(
      picker().getByText("Services with recorded usage in this period."),
    ).toBeVisible();
    expect(option("Free service")).toHaveTextContent("free-service");
    expect(
      picker().queryByRole("checkbox", { name: /^Unused service/ }),
    ).not.toBeInTheDocument();
    await selectServices(["Free service"]);
    expect(screen.queryByText("Example LLM")).not.toBeInTheDocument();
    expect(
      screen.getByText("Free", { exact: true, selector: ".usage-status" }),
    ).toBeVisible();
    expect(servicesChip()).toHaveTextContent("Service: Free service");
    expect(history.location.pathname).toBe(pageRoute);
    expect(servicesParam(history)).toEqual(["free-service"]);
    await act(() => history.back());
    await waitFor(() =>
      expect(
        screen.getByText("Example LLM", {
          selector: ".expandable-name > strong",
        }),
      ).toBeVisible(),
    );
  });
  it("selects several services and withholds global totals for any selection", async () => {
    mocks.usage.mockReturnValue(
      query(usage([...twoServiceRows(), row({ service_slug: "third" })])),
    );
    const { history } = await renderPage();
    const totals = () =>
      document
        .querySelector(".all-metrics")
        ?.textContent?.includes("Reported requests");
    expect(totals()).toBe(true);
    await userEvent.click(serviceFilter());
    await selectServices(["Free service", "Example LLM"]);
    expect(servicesChip("Free service")).toHaveTextContent(
      "Service: Free service",
    );
    expect(servicesChip("Example LLM")).toHaveTextContent(
      "Service: Example LLM",
    );
    expect(serviceFilter()).toHaveTextContent("2 selected");
    expect(servicesParam(history)).toEqual(["free-service", "example-llm"]);
    expect(screen.queryByText("third")).not.toBeInTheDocument();
    expect(totals()).toBe(false);
  });
  it("updates service selection immediately and clears it with Clear and Done", async () => {
    mocks.usage.mockReturnValue(query(usage(twoServiceRows())));
    const { history } = await renderPage(
      `/billing?tab=usage&services=${encodeURIComponent('["free-service"]')}`,
    );
    await userEvent.click(serviceFilter());
    await userEvent.click(option("Example LLM"));
    await waitFor(() =>
      expect(servicesParam(history)).toEqual(["free-service", "example-llm"]),
    );
    expect(option("Example LLM")).toBeChecked();
    expect(
      picker().queryByRole("button", { name: "Apply" }),
    ).not.toBeInTheDocument();
    expect(
      picker().queryByRole("button", { name: "Cancel" }),
    ).not.toBeInTheDocument();
    await userEvent.click(picker().getByRole("button", { name: "Done" }));
    expect(history.location.pathname).toBe(pageRoute);
    await userEvent.click(serviceFilter());
    expect(option("Example LLM")).toBeChecked();
    await userEvent.click(picker().getByRole("button", { name: "Clear" }));
    await waitFor(() => expect(servicesParam(history)).toBeUndefined());
    expect(option("Example LLM")).not.toBeChecked();
    await userEvent.click(picker().getByRole("button", { name: "Done" }));
    expect(servicesChip()).toBeNull();
    expect(serviceFilter()).toHaveTextContent("All");
  });
  it("reopens the picker from a chip and removes one service without clearing others", async () => {
    mocks.usage.mockReturnValue(
      query(usage([...twoServiceRows(), row({ service_slug: "third" })])),
    );
    const { history } = await renderPage();
    await userEvent.click(serviceFilter());
    await selectServices(["Free service", "Example LLM"]);
    await userEvent.click(servicesChip("Free service")!);
    expect(option("Free service")).toBeChecked();
    expect(option("Example LLM")).toBeChecked();
    expect(option("third")).not.toBeChecked();
    await userEvent.type(
      picker().getByRole("textbox", { name: "Search services" }),
      "third",
    );
    expect(picker().getAllByRole("checkbox")).toHaveLength(1);
    await userEvent.click(option("third"));
    await waitFor(() =>
      expect(servicesParam(history)).toEqual([
        "free-service",
        "example-llm",
        "third",
      ]),
    );
    await userEvent.click(picker().getByRole("button", { name: "Done" }));
    await userEvent.click(
      screen.getByRole("button", { name: "Remove service: third" }),
    );
    await waitFor(() =>
      expect(servicesParam(history)).toEqual(["free-service", "example-llm"]),
    );
    expect(servicesChip("Free service")).toBeVisible();
    expect(servicesChip("Example LLM")).toBeVisible();
    await userEvent.click(serviceFilter());
    expect(
      picker().getByRole("textbox", { name: "Search services" }),
    ).toHaveValue("");
    expect(option("third")).not.toBeChecked();
    await userEvent.click(picker().getByRole("button", { name: "Done" }));
    await userEvent.click(
      screen.getAllByRole("button", { name: "Clear filters" })[0]!,
    );
    await waitFor(() => expect(servicesParam(history)).toBeUndefined());
  });
  it.each([
    ["/billing?tab=usage&service=all", undefined],
    ["/billing?tab=usage&service=free-service", ["free-service"]],
    [
      `/billing?tab=usage&service=free-service&services=${encodeURIComponent('["example-llm"]')}`,
      ["example-llm"],
    ],
  ])("migrates the legacy service param from %s", async (url, expected) => {
    mocks.usage.mockReturnValue(query(usage(twoServiceRows())));
    const { history } = await renderPage(url);
    await waitFor(() =>
      expect(history.location.search).not.toMatch(/(?:service|panelService)=/),
    );
    expect(servicesParam(history)).toEqual(expected);
  });
  it("prunes unknown services only after the usage query settles", async () => {
    let state: ReturnType<typeof query> = {
      ...query(undefined),
      isLoading: true,
      isSuccess: false,
    };
    mocks.usage.mockImplementation(() => state);
    const url = `/billing?tab=usage&services=${encodeURIComponent('["example-llm","gone"]')}`;
    const { history } = await renderPage(url);
    const rerender = async () => {
      await userEvent.click(screen.getByRole("tab", { name: "Billing" }));
      await userEvent.click(screen.getByRole("tab", { name: "Usage" }));
    };
    expect(servicesParam(history)).toEqual(["example-llm", "gone"]);
    state = { ...query(usage([row()])), isFetching: true };
    await rerender();
    expect(servicesParam(history)).toEqual(["example-llm", "gone"]);
    state = query(usage([row()]));
    await rerender();
    await waitFor(() =>
      expect(servicesParam(history)).toEqual(["example-llm"]),
    );
  });
  it("resets an unavailable service after a time change while keeping history independent", async () => {
    mocks.usage.mockImplementation((period) =>
      query(usage(period === "24h" ? [] : [row()])),
    );
    const { history } = await renderPage(
      `/billing?tab=usage&services=${encodeURIComponent('["example-llm"]')}&period=7d`,
    );
    await select("Time range", "Last 24 hours");
    await waitFor(() => expect(servicesParam(history)).toBeUndefined());
    expect(screen.getByText("No usage in this period.")).toBeVisible();
    await userEvent.click(screen.getByRole("tab", { name: "Billing" }));
    expect(
      screen.getByRole("combobox", { name: "Top-up history period" }),
    ).toHaveTextContent("Last 30 days");
  });
  it("opens Add credits once from action=topup and clears the param", async () => {
    const { history } = await renderPage("/billing?tab=billing&action=topup");
    expect(await screen.findByRole("dialog")).toBeVisible();
    await waitFor(() =>
      expect(history.location.search).not.toMatch(/(?:action|panelAction)=/),
    );
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    await act(() => history.back());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
  it.each([
    "/billing?tab=billing&service=all&action=topup",
    `/billing?tab=billing&services=${encodeURIComponent('["gone"]')}&action=topup`,
  ])(
    "keeps the top-up deep link through filter cleanup when usage settles before the wallet (%s)",
    async (url) => {
      let wallet: ReturnType<typeof query> = {
        ...query(undefined),
        isLoading: true,
        isSuccess: false,
      };
      const listeners = new Set<() => void>();
      mocks.wallet.mockImplementation(() =>
        useSyncExternalStore(
          (listener) => {
            listeners.add(listener);
            return () => listeners.delete(listener);
          },
          () => wallet,
        ),
      );
      const { history } = await renderPage(url);
      // Usage has settled, so legacy/stale filters are cleaned up first...
      await waitFor(() =>
        expect(history.location.search).not.toMatch(/(?:service|services|panelService|panelServices)=/),
      );
      expect(history.location.search).toContain(pageRoute === "/assistant" ? "panelAction=topup" : "action=topup");
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      // ...and the wallet arrives later: Add credits opens exactly once.
      act(() => {
        wallet = query(billingWallet());
        listeners.forEach((listener) => listener());
      });
      expect(
        await screen.findByRole("dialog", { name: "Add credits" }),
      ).toBeVisible();
      await waitFor(() =>
        expect(history.location.search).not.toMatch(/(?:action|panelAction)=/),
      );
      await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
      await waitFor(() =>
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
      );
      await act(() => history.back());
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    },
  );
  it("never replays a pending top-up link after the user navigates away, including on Back", async () => {
    let wallet: ReturnType<typeof query> = {
      ...query(undefined),
      isLoading: true,
      isSuccess: false,
    };
    const listeners = new Set<() => void>();
    mocks.wallet.mockImplementation(() =>
      useSyncExternalStore(
        (listener) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        },
        () => wallet,
      ),
    );
    const { history } = await renderPage("/billing?tab=billing&action=topup");
    // The user picks Usage while the wallet is still loading.
    await userEvent.click(screen.getByRole("tab", { name: "Usage" }));
    await waitFor(() => expect(history.location.search).toContain(pageRoute === "/assistant" ? "panelTab=usage" : "tab=usage"));
    expect(history.location.pathname).toBe(pageRoute);
    expect(history.location.search).not.toMatch(/(?:action|panelAction)=/);
    act(() => {
      wallet = query(billingWallet());
      listeners.forEach((listener) => listener());
    });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    // Back lands on the original entry, whose pending action was consumed.
    await act(() => history.back());
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Billing" })).toHaveAttribute(
        "data-state",
        "active",
      ),
    );
    expect(history.location.pathname).toBe(pageRoute);
    expect(history.location.search).not.toMatch(/(?:action|panelAction)=/);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await act(() => history.forward());
    await act(() => history.back());
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
  it("lands on the Billing tab without a dialog when top-up is impossible", async () => {
    mocks.usage.mockReturnValue(
      query({
        ...usage(),
        billing: { ...usage().billing, charging_enabled: false },
      }),
    );
    const { history } = await renderPage("/billing?tab=usage&action=topup");
    await waitFor(() =>
      expect(history.location.search).not.toMatch(/(?:action|panelAction)=/),
    );
    expect(screen.getByRole("tab", { name: "Billing" })).toHaveAttribute(
      "data-state",
      "active",
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
  it("does not turn missing costs into zero or count free records as pending", async () => {
    mocks.usage.mockReturnValue(
      query(
        usage([
          row({ estimated_credits_micros: null }),
          row(),
          row({
            billable: false,
            lago_acked: false,
            estimated_credits_micros: 0,
            wallet_credits_micros: 0,
            grant_credits_micros: 0,
            allowance_credits_micros: 0,
          }),
        ]),
      ),
    );
    await renderPage();
    const spend = screen.getByRole("heading", { name: "Spend" }).parentElement!;
    expect(
      within(spend).getByText("≥ 0.00244", { exact: false }),
    ).toBeVisible();
    expect(
      within(spend).getByText(
        "Estimated usage cost · lower bound, 1 record unpriced",
      ),
    ).toBeVisible();
    expect(within(spend).queryByText("Unavailable")).not.toBeInTheDocument();
    expect(
      screen.getByText("≥ 0.00244", { selector: ".row-credit", exact: false }),
    ).toBeVisible();
    expect(
      screen.getAllByText(
        /^1 charged record was metered under a price that is no longer available, so its gross, wallet and allowance costs cannot be estimated\. Amounts marked ≥ are lower bounds\./,
      ).length,
    ).toBeGreaterThan(0);
    await userEvent.click(
      screen.getByText("Example LLM", {
        selector: ".expandable-name > strong",
      }),
    );
    expect(
      screen.queryByRole("tab", { name: /^Records/ }),
    ).not.toBeInTheDocument();
    const detail = document.querySelector(".detailed-usage")!;
    expect(detail.querySelector(".funding-visual")).toHaveTextContent(
      "Some funding values are unavailable.",
    );
    expect(screen.getByText("Includes free usage")).toBeVisible();
    expect(
      screen.getByText("Acknowledged", { selector: ".usage-status" }),
    ).toBeVisible();
    expect(
      screen.queryByText("Pending", { exact: true }),
    ).not.toBeInTheDocument();
  });
  it("switches the optional breakdown between services, models, and agents and searches its groups", async () => {
    mocks.usage.mockReturnValue(
      query(
        usage([
          row({
            model: "Model A",
            api_key_id: "agent-a",
            api_key_name: "Research agent",
            estimated_credits: "0.000000000012",
            wallet_credits: "0.000000000012",
            grant_credits: "0",
            allowance_credits: "0",
          }),
          row({
            model: "Model B",
            api_key_id: "agent-b",
            api_key_name: "Writing agent",
            estimated_credits: "0.000000000003",
            wallet_credits: "0.000000000003",
            grant_credits: "0",
            allowance_credits: "0",
          }),
        ]),
      ),
    );
    await renderPage();
    await select("Group by", "Model");
    expect(
      screen.getByText("Model A", { selector: ".expandable-name > strong" }),
    ).toBeVisible();
    expect(
      screen.getByText("Model B", { selector: ".expandable-name > strong" }),
    ).toBeVisible();
    await select("Group by", "Agent");
    await userEvent.type(
      screen.getByRole("textbox", { name: "Search usage groups" }),
      "Research",
    );
    expect(
      screen.getByText("Research agent", {
        selector: ".expandable-name > strong",
      }),
    ).toBeVisible();
    expect(screen.queryByText("Writing agent")).not.toBeInTheDocument();
  });
  it("shows daily request and error lines and selects one agent independently of billing filters", async () => {
    mocks.activity.mockReturnValue(
      query([
        billingAgentUsage(),
        billingAgentUsage({
          api_key_id: "writer",
          api_key_name: "Writing agent",
        }),
      ]),
    );
    await renderPage();
    expect(
      screen.getByRole("img", {
        name: "Daily activity: 58 requests and 4 errors across 7 UTC days",
      }),
    ).toBeVisible();
    await select("Activity agent", "Writing agent");
    expect(
      screen.getByRole("img", {
        name: "Daily activity: 29 requests and 2 errors across 7 UTC days",
      }),
    ).toBeVisible();
    await select("Activity time range", "Last 30 days");
    expect(mocks.activity).toHaveBeenLastCalledWith(30);
    await userEvent.click(screen.getByText("View daily values"));
    const table = screen.getByRole("table", {
      name: "Agent activity per UTC day",
    });
    expect(within(table).getAllByRole("row")).toHaveLength(8);
    expect(within(table).getAllByRole("row")[3]).toHaveTextContent(
      "2026-10-0381",
    );
  });
  it("shows the spend as unavailable when no charged record is priced", async () => {
    mocks.usage.mockReturnValue(
      query(
        usage([
          row({ estimated_credits_micros: null }),
          // A free row's zero cost must not make the charged total known.
          row({
            service_slug: "free-service",
            billable: false,
            lago_acked: false,
            estimated_credits_micros: 0,
            wallet_credits_micros: 0,
            grant_credits_micros: 0,
            allowance_credits_micros: 0,
          }),
        ]),
      ),
    );
    await renderPage();
    const spend = screen.getByRole("heading", { name: "Spend" }).parentElement!;
    expect(spend.querySelector("strong")).toHaveTextContent(
      "Unavailable credits",
    );
    expect(within(spend).getByText("Estimated usage cost")).toBeVisible();
    expect(within(spend).queryByText(/lower bound/)).not.toBeInTheDocument();
  });
  it("shows usage and history errors as retryable failures instead of empty results", async () => {
    mocks.usage.mockReturnValue(query(undefined, new Error("Usage failed")));
    mocks.history.mockReturnValue(
      query(undefined, new Error("History failed")),
    );
    await renderPage();
    expect(screen.getByText("Usage failed")).toBeVisible();
    expect(
      screen.queryByText("No usage in this period."),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(mocks.usage().refetch).toHaveBeenCalled();
    await userEvent.click(screen.getByRole("tab", { name: "Billing" }));
    expect(screen.getByText("Failed to load top-up history.")).toBeVisible();
    expect(screen.queryByText("No top-ups yet.")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add credits" })).toBeDisabled();
  });
  it("keeps wallet provisioning connected to its real mutation", async () => {
    mocks.wallet.mockReturnValue(
      query(
        undefined,
        new ApiError(503, {
          error: "billing",
          error_code: 11301,
          message: "No wallet",
        }),
      ),
    );
    await renderPage("/billing");
    await userEvent.click(
      screen.getByRole("button", { name: "Provision Wallet" }),
    );
    expect(mocks.provision).toHaveBeenCalledWith({});
  });
});

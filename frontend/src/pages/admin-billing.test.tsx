import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useSyncExternalStore } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  BillingConfig,
  BillingRate,
  BillingService,
} from "@/schemas/admin-billing";
import { AdminBillingPage } from "./admin-billing";

const mock = vi.hoisted(() => ({
  search: {} as Record<string, unknown>,
  listeners: new Set<() => void>(),
  navigate: vi.fn(),
  user: { is_admin: true, role: "admin" } as Record<string, unknown>,
  resync: vi.fn(),
  updateConfig: vi.fn(),
  updateService: vi.fn(),
  overview: [] as unknown[],
  rates: [] as unknown[],
  config: null as unknown,
}));

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  useSearch: () =>
    useSyncExternalStore(
      (listener) => {
        mock.listeners.add(listener);
        return () => {
          mock.listeners.delete(listener);
        };
      },
      () => mock.search,
    ),
  useNavigate: () => (options: { search: Record<string, unknown> }) => {
    mock.navigate(options);
    mock.search = options.search;
    mock.listeners.forEach((listener) => listener());
  },
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (state: unknown) => unknown) =>
    select({ user: mock.user }),
}));
vi.mock("@/hooks/use-endpoints", () => ({
  useEndpoints: () => ({ data: [] }),
}));
vi.mock("@/hooks/use-services", () => ({
  useService: () => ({
    data: {
      id: "svc-1",
      billing: {
        byok_pricing: {
          metric: "requests",
          credits_per_unit: "0.25",
          lago_metric_code: "svc_byok",
          sync_status: "synced",
        },
      },
    },
    isLoading: false,
    error: null,
  }),
}));
vi.mock("@/hooks/use-admin-billing", () => ({
  useAdminBillingOverview: () => ({
    data: mock.overview,
    isLoading: false,
    error: null,
  }),
  useAdminBillingRates: () => ({
    data: mock.rates,
    isLoading: false,
    error: null,
  }),
  useAdminBillingConfig: () => ({
    data: mock.config,
    isLoading: false,
    error: null,
  }),
  useResyncAdminBillingService: () => ({
    mutate: mock.resync,
    isPending: false,
  }),
  useUpdateAdminBillingConfig: () => ({
    mutate: mock.updateConfig,
    isPending: false,
    error: null,
  }),
  useUpdateAdminBillingService: () => ({
    mutate: mock.updateService,
    isPending: false,
    error: null,
  }),
}));

const service: BillingService = {
  id: "svc-1",
  slug: "svc",
  name: "Search API",
  is_active: true,
  effective_metric: "requests",
  allowance_metrics: ["requests"],
  effective: {
    fallback_metric: "requests",
    resale: { configured: false, applied: false, source: "free" },
    classes: [
      {
        credential_classes: ["user_owned", "node_managed"],
        lane: "byok",
        platform_charged: true,
        source: "service",
        reason: "lane_synced",
        prices: [
          {
            layer: "platform",
            metric: "requests",
            operation: null,
            lago_metric_code: "svc_byok",
            credits_per_unit: "0.25",
            source: "service",
            origin: "lane_primary",
            sync_status: "synced",
            sync_error: null,
            cached_rate: {
              credits_per_unit: "0.25",
              synced_at: "2026-10-01T00:00:00Z",
              stale: true,
              retired: false,
            },
          },
        ],
        pending: [
          {
            layer: "platform",
            metric: "images",
            operation: null,
            lago_metric_code: "svc_byok_images",
            credits_per_unit: "2",
            source: "free",
            origin: "lane_component",
            sync_status: "failed",
            sync_error: "lago rejected the charge",
            cached_rate: null,
          },
        ],
      },
      {
        credential_classes: ["no_auth"],
        lane: null,
        platform_charged: false,
        source: "free",
        reason: "no_auth",
        prices: [],
        pending: [],
      },
    ],
  },
};

const config: BillingConfig = {
  billing_enabled: true,
  lago_configured: true,
  resale_enabled: false,
  default_overdraft_cap_credits: {
    effective: 12,
    source: "global",
    global: 12,
    env: 5,
  },
  max_default_overdraft_cap_credits: 1000,
  environment: [
    {
      key: "LAGO_PLAN_CODE",
      value: "starter",
      source: "env",
      secret_set: null,
    },
    { key: "LAGO_API_KEY", value: null, source: "env", secret_set: true },
  ],
};

const rate: BillingRate = {
  id: "platform_requests:*",
  lago_metric_code: "platform_requests",
  model: null,
  credits_per_unit: "0.000000000001",
  synced_at: "2026-10-01T00:00:00Z",
  stale_after: "2026-10-01T00:10:00Z",
  stale: false,
  retired_at: "2026-10-02T00:00:00Z",
};

beforeEach(() => {
  mock.search = {};
  mock.navigate.mockReset();
  mock.resync.mockReset();
  mock.updateConfig.mockReset();
  mock.updateService.mockReset();
  mock.user = { is_admin: true, role: "admin" };
  mock.overview = [service];
  mock.rates = [rate];
  mock.config = config;
});

describe("AdminBillingPage", () => {
  it("shows every class's effective price with its source, sync state and reason", () => {
    render(<AdminBillingPage />);
    const row = screen.getByText("Search API").closest("tr")!;
    expect(
      within(row).getByText("Own key, Node credential"),
    ).toBeInTheDocument();
    expect(
      within(row).getByText(/0\.25 credits \/ request · lane price/),
    ).toBeInTheDocument();
    expect(within(row).getByText("Stale rate")).toBeInTheDocument();
    expect(within(row).getByText("Synced")).toBeInTheDocument();
    expect(
      within(row).getByText(/2 credits \/ image · lane component/),
    ).toBeInTheDocument();
    expect(
      within(row).getByText("lago rejected the charge"),
    ).toBeInTheDocument();
    expect(
      within(row).getByText(/never priced once lanes exist/),
    ).toBeInTheDocument();
    expect(within(row).getAllByText("Service").length).toBeGreaterThan(0);
    expect(within(row).getAllByText("Free").length).toBeGreaterThan(0);
  });

  it("drives the selected tab from the URL", async () => {
    const user = userEvent.setup();
    render(<AdminBillingPage />);
    await user.click(screen.getByRole("tab", { name: "Rate cache" }));
    expect(mock.navigate).toHaveBeenCalledWith(
      expect.objectContaining({
        to: "/admin/billing",
        search: { tab: "rates" },
      }),
    );
    expect(await screen.findByText("platform_requests")).toBeInTheDocument();
    expect(screen.getByText("0.000000000001")).toBeInTheDocument();
    expect(screen.getByText("Retired")).toBeInTheDocument();
  });

  it("hides write controls from operators", () => {
    mock.user = { is_admin: false, is_operator: true, role: "operator" };
    render(<AdminBillingPage />);
    expect(screen.queryByRole("button", { name: "Resync" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Edit prices" })).toBeNull();
  });

  it("keeps the settings read-only for operators", async () => {
    mock.user = { is_admin: false, is_operator: true, role: "operator" };
    mock.search = { tab: "settings" };
    render(<AdminBillingPage />);
    expect(await screen.findByText("Global defaults")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Save global defaults" }),
    ).toBeNull();
    expect(
      screen.queryByLabelText("Global overdraft cap (credits)"),
    ).toBeNull();
  });

  it("resyncs a service through the admin endpoint", async () => {
    const user = userEvent.setup();
    render(<AdminBillingPage />);
    await user.click(screen.getByRole("button", { name: "Resync" }));
    expect(mock.resync).toHaveBeenCalledWith("svc-1", expect.anything());
  });

  it("prefills the global cap, mirrors env values and never shows secrets", async () => {
    const user = userEvent.setup();
    mock.search = { tab: "settings" };
    render(<AdminBillingPage />);
    const input = await screen.findByLabelText(
      "Global overdraft cap (credits)",
    );
    expect(input).toHaveValue("12");
    expect(screen.getByText("starter")).toBeInTheDocument();
    expect(screen.getByText("Set (hidden)")).toBeInTheDocument();
    expect(screen.getAllByText("Global default").length).toBeGreaterThan(0);

    await user.clear(input);
    await user.type(input, "4000");
    await user.click(
      screen.getByRole("button", { name: "Save global defaults" }),
    );
    expect(await screen.findByText(/between 0 and 1000/)).toBeInTheDocument();
    expect(mock.updateConfig).not.toHaveBeenCalled();

    await user.clear(input);
    await user.type(input, "40");
    await user.click(
      screen.getByRole("button", { name: "Save global defaults" }),
    );
    await waitFor(() =>
      expect(mock.updateConfig).toHaveBeenCalledWith(
        { default_overdraft_cap_credits: 40 },
        expect.anything(),
      ),
    );

    await user.clear(input);
    await user.click(
      screen.getByRole("button", { name: "Save global defaults" }),
    );
    await waitFor(() =>
      expect(mock.updateConfig).toHaveBeenLastCalledWith(
        { default_overdraft_cap_credits: null },
        expect.anything(),
      ),
    );
  });

  it("edits lane prices inline and saves only the changed lane", async () => {
    const user = userEvent.setup();
    render(<AdminBillingPage />);
    await user.click(screen.getByRole("button", { name: "Edit prices" }));
    const prices = await screen.findAllByLabelText("Credits per unit");
    await user.clear(prices[0]!);
    await user.type(prices[0]!, "0.5");
    await user.click(screen.getByRole("button", { name: "Save prices" }));
    await waitFor(() => expect(mock.updateService).toHaveBeenCalledTimes(1));
    const [payload] = mock.updateService.mock.calls[0]!;
    expect(payload).toEqual({
      serviceId: "svc-1",
      billing: {
        byok_pricing: { metric: "requests", credits_per_unit: "0.5" },
      },
    });
  });
});

import "@/i18n";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { AvailableBenefits } from "./available-benefits";
import { availableCreditTotal } from "@/lib/credits";
import {
  billingWallet,
  billingGrant,
  billingAllowance,
} from "@/test/billing-fixture";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { ApiError } from "@/lib/api-client";
const { get } = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  api: { get },
}));
const response = (page = 1) => ({
  summary: {
    remaining: "20",
    unreserved: "12.000000000001",
    held: "3",
    pending_activation: "5",
    pending_reclaim: "0",
    expired_awaiting_cleanup: "0",
    restricted: "4",
    generated_at: "2026-10-01T00:00:00Z",
  },
  grants: [
    {
      ...billingGrant({ id: `grant-${page}`, expires_at: null }),
      available: "12.000000000001",
      held: "3",
    },
  ],
  allowances: [billingAllowance("requests")],
  total_grants: 51,
  page,
  per_page: 50,
});
function mount() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <AvailableBenefits />
      </TooltipProvider>
    </QueryClientProvider>,
  );
  return client;
}
beforeEach(() => {
  useAuthStore.setState({ user: { id: "account-a" } as User });
  get.mockReset();
  get.mockImplementation((path: string) =>
    Promise.resolve(
      path === "/billing/wallet"
        ? { ...billingWallet(), available: "2.000000000001" }
        : response(path.includes("page=2") ? 2 : 1),
    ),
  );
});
it("isolates organization benefits and clears personal totals while the selected account loads", async () => {
  const org = "11111111-1111-4111-8111-111111111111";
  let resolveOrg: (value: unknown) => void = () => {};
  get.mockImplementation((path: string) => {
    if (path.includes(`owner_id=${org}`)) {
      return path.startsWith("/billing/wallet")
        ? Promise.resolve({ ...billingWallet(), owner_id: org, available: "5" })
        : new Promise((resolve) => {
            resolveOrg = resolve;
          });
    }
    return Promise.resolve(
      path === "/billing/wallet"
        ? { ...billingWallet(), available: "2.000000000001" }
        : response(),
    );
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  function View({ ownerId }: { ownerId?: string }) {
    return (
      <QueryClientProvider client={client}>
        <TooltipProvider>
          <AvailableBenefits key={ownerId ?? "personal"} ownerId={ownerId} />
        </TooltipProvider>
      </QueryClientProvider>
    );
  }
  const view = render(<View />);
  await screen.findByTitle("14.000000000002");
  view.rerender(<View ownerId={org} />);
  expect(screen.queryByTitle("14.000000000002")).not.toBeInTheDocument();
  await waitFor(() =>
    expect(get).toHaveBeenCalledWith(
      `/billing/benefits?page=1&owner_id=${org}`,
    ),
  );
  await act(async () =>
    resolveOrg({
      ...response(),
      summary: { ...response().summary, unreserved: "7" },
    }),
  );
  expect(await screen.findByTitle("12")).toHaveTextContent("12 credits");
  view.rerender(<View />);
  expect(await screen.findByTitle("14.000000000002")).toHaveTextContent(
    "14 credits",
  );
});
it("shows zero spendable credits when an organization has no provisioned wallet", async () => {
  get.mockImplementation((path: string) =>
    Promise.resolve(path.startsWith("/billing/wallet") ? null : response()),
  );
  mount();
  expect(await screen.findByTitle("0")).toHaveTextContent("0 credits");
  expect(screen.getByRole("status")).toHaveTextContent("Provision a wallet");
});
it("adds exact wallet and spendable grants, with native allowance units and full-population totals", async () => {
  mount();
  expect(await screen.findByTitle("14.000000000002")).toHaveTextContent(
    "14 credits",
  );
  expect(availableCreditTotal("2.000000000001", "12.000000000001")).toBe(
    "14.000000000002",
  );
  const disclosure = screen.getByRole("button", { name: "View breakdown" });
  expect(disclosure).toHaveAttribute("aria-expanded", "false");
  expect(screen.getByText("Free allowances")).not.toBeVisible();
  expect(screen.getByText("Pending activation")).not.toBeVisible();
  expect(
    screen.getByRole("heading", { name: "Available credits" }),
  ).toBeVisible();
  disclosure.focus();
  await userEvent.keyboard("{Enter}");
  expect(
    screen.getByRole("button", { name: "Hide breakdown" }),
  ).toHaveAttribute("aria-expanded", "true");
  expect(
    document.getElementById(disclosure.getAttribute("aria-controls")!),
  ).toBeVisible();
  expect(screen.getByText("Free allowances")).toBeVisible();
  expect(screen.getByText("Pending activation")).toBeVisible();
  expect(screen.getByText(/Available: 12 credits · Held:/)).toBeVisible();
});
it("uses singular credit and grant labels in the headline and expanded history", async () => {
  const data = response();
  get.mockImplementation((path: string) =>
    Promise.resolve(
      path === "/billing/wallet"
        ? { ...billingWallet(), available: "0" }
        : {
            ...data,
            summary: { ...data.summary, unreserved: "1" },
            total_grants: 1,
            grants: [{ ...data.grants[0], available: "1", amount: "1" }],
          },
    ),
  );
  mount();
  expect(await screen.findByTitle("1")).toHaveTextContent("1 credit");
  await userEvent.click(screen.getByText("View breakdown"));
  expect(screen.getByText(/1 grant · page 1\./)).toBeVisible();
  expect(screen.getByText(/Granted 1 credit ·/)).toBeVisible();
});
it("preserves the expanded breakdown and authoritative headline while the next page loads", async () => {
  let resolve: (v: unknown) => void = () => {};
  get.mockImplementation((path: string) =>
    path.includes("page=2")
      ? new Promise((r) => {
          resolve = r;
        })
      : Promise.resolve(
          path === "/billing/wallet" ? billingWallet() : response(),
        ),
  );
  mount();
  await screen.findByText("View breakdown");
  await userEvent.click(screen.getByText("View breakdown"));
  await userEvent.click(screen.getByRole("button", { name: "Next grants" }));
  expect(screen.getByText("Individual grants")).toBeVisible();
  expect(screen.getByRole("button", { name: "Next grants" })).toBeDisabled();
  await act(async () => resolve(response(2)));
  await waitFor(() =>
    expect(screen.getByText(/51 grants · page 2/)).toBeVisible(),
  );
  expect(screen.getByText("Individual grants")).toBeVisible();
});
it("does not present suspended benefits as spendable", async () => {
  get.mockImplementation((path: string) =>
    Promise.resolve(
      path === "/billing/wallet"
        ? { ...billingWallet(), suspended: true }
        : response(),
    ),
  );
  mount();
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Usage is suspended",
  );
  expect(screen.getByTitle("0")).toHaveTextContent("0 credits");
  expect(screen.getByTitle("0")).toBeVisible();
});
it("shows a failed refresh as stale and does not silently discard it", async () => {
  const client = mount();
  await screen.findByTitle("14.000000000002");
  get.mockRejectedValue(new Error("offline"));
  await act(() => client.invalidateQueries());
  expect(
    await screen.findByText(
      "Showing previously loaded balances. Refresh failed.",
    ),
  ).toBeVisible();
});
it("clears benefit details after authorization is lost", async () => {
  const client = mount();
  await screen.findByText("View breakdown");
  get.mockRejectedValue(
    new ApiError(403, {
      error: "forbidden",
      error_code: 1003,
      message: "Forbidden",
    }),
  );
  await act(() => client.invalidateQueries());
  expect(await screen.findByText("Unavailable")).toBeVisible();
  expect(screen.queryByText("View breakdown")).not.toBeInTheDocument();
});

it("keeps grants spendable when the wallet is negative and shows the signed wallet separately", async () => {
  get.mockImplementation((path: string) =>
    Promise.resolve(
      path === "/billing/wallet"
        ? { ...billingWallet(), available: "-8" }
        : response(),
    ),
  );
  mount();
  expect(await screen.findByTitle("12.000000000001")).toHaveTextContent(
    "12 credits",
  );
  expect(screen.getByText("-8 credits")).not.toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "View breakdown" }));
  expect(screen.getByText("-8 credits")).toBeVisible();
  expect(availableCreditTotal("-8", "12.000000000001")).toBe("12.000000000001");
});
it("never combines a new account's grants with the previous account's cached wallet", async () => {
  mount();
  await screen.findByTitle("14.000000000002");
  let resolveWallet: (v: unknown) => void = () => {};
  get.mockImplementation((path: string) =>
    path === "/billing/wallet"
      ? new Promise((r) => {
          resolveWallet = r;
        })
      : Promise.resolve(response()),
  );
  await act(async () =>
    useAuthStore.setState({ user: { id: "account-b" } as User }),
  );
  expect(screen.queryByTitle("14.000000000002")).not.toBeInTheDocument();
  expect(screen.getByLabelText("Loading available credits")).toBeVisible();
  await act(async () =>
    resolveWallet({
      ...billingWallet(),
      owner_id: "account-b",
      available: "40",
    }),
  );
  expect(await screen.findByTitle("52.000000000001")).toBeVisible();
});
it("refreshes wallet and benefits together every 30 seconds on the active page", async () => {
  vi.useFakeTimers();
  try {
    await act(async () => {
      mount();
    });
    get.mockClear();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(get).toHaveBeenCalledWith("/billing/wallet");
    expect(get).toHaveBeenCalledWith("/billing/benefits?page=1");
  } finally {
    vi.useRealTimers();
  }
});

it("keeps wallet actions available while benefits load without showing a partial total", async () => {
  get.mockImplementation((path: string) =>
    path === "/billing/wallet"
      ? Promise.resolve(billingWallet())
      : new Promise(() => {}),
  );
  const client = new QueryClient();
  render(
    <QueryClientProvider client={client}>
      <AvailableBenefits actions={<button>Add credits</button>} />
    </QueryClientProvider>,
  );
  expect(screen.getByRole("button", { name: "Add credits" })).toBeVisible();
  expect(screen.getByLabelText("Loading available credits")).toBeVisible();
  expect(screen.queryByTitle("95")).not.toBeInTheDocument();
});

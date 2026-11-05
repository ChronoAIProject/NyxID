import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import { FreeGrantsReport } from "./free-grants-report";
import { freeGrantParams } from "@/hooks/use-free-grants-report";
import { EMPTY_FILTERS } from "@/lib/usage-analytics";
import { ApiError } from "@/lib/api-client";
const { get } = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  api: { get },
}));
const window = { from: "2026-10-01T00:00:00Z", to: "2026-10-02T00:00:00Z" };
const response = (page = 1) => ({
  window,
  generated_at: window.to,
  issued: "123.000000000001",
  grants: 70,
  total_accounts: 51,
  page,
  per_page: 50,
  accounts: [
    {
      owner_id: `account-${page}`,
      label: page === 1 ? "Ada" : "Research",
      account_kind: page === 1 ? "person" : "org",
      issued: "1.000000000001",
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
beforeEach(() => {
  get.mockReset();
  get.mockResolvedValue(response());
});
it("keeps full-population issuance distinct from loaded accounts and usage filters", async () => {
  mount();
  expect(await screen.findByTitle("123.000000000001")).toHaveTextContent(
    "123 credits",
  );
  expect(
    screen.getByText(/70 grants across 51 billing accounts/),
  ).toBeVisible();
  await userEvent.click(screen.getByText("Issued by account"));
  expect(screen.getByText("Personal account · 1 grant")).toBeVisible();
  expect(screen.getByTitle("1.000000000001")).toHaveTextContent(
    "1 credit issued",
  );
  const params = freeGrantParams({
    ...EMPTY_FILTERS,
    owners: ["z", "a"],
    actors: ["actor"],
    services: ["service"],
  });
  expect(params.get("owners")).toBe("a,z");
  expect([...params.keys()]).toEqual(["period", "owners"]);
});
it("uses singular labels for one credit, grant, and billing account", async () => {
  const data = response();
  get.mockResolvedValue({
    ...data,
    issued: "1",
    grants: 1,
    total_accounts: 1,
    accounts: [{ ...data.accounts[0], issued: "1" }],
  });
  mount();
  expect(await screen.findByText("1 credit", { exact: true })).toBeVisible();
  expect(screen.getByText(/1 grant across 1 billing account ·/)).toBeVisible();
  await userEvent.click(screen.getByText("Issued by account"));
  expect(screen.getByText("Personal account · 1 grant")).toBeVisible();
  expect(screen.getByText("1 credit issued")).toBeVisible();
});
it("preserves the expanded account list while paging and pins the effective UTC window", async () => {
  let resolve: (v: unknown) => void = () => {};
  get.mockImplementation((path: string) =>
    path.includes("page=2")
      ? new Promise((r) => {
          resolve = r;
        })
      : Promise.resolve(response()),
  );
  mount();
  await screen.findByText("Issued by account");
  await userEvent.click(screen.getByText("Issued by account"));
  await userEvent.click(screen.getByRole("button", { name: "Next accounts" }));
  expect(screen.getByText("Ada")).toBeVisible();
  expect(screen.getByRole("button", { name: "Next accounts" })).toBeDisabled();
  const params = new URLSearchParams(get.mock.calls.at(-1)![0].split("?")[1]);
  expect(params.get("from")).toBe(window.from);
  expect(params.get("to")).toBe(window.to);
  expect(params.has("period")).toBe(false);
  await act(async () => resolve(response(2)));
  expect(await screen.findByText("Research")).toBeVisible();
  expect(screen.getByText(/Organization/)).toBeVisible();
  expect(screen.getByText("Page 2 of 2")).toBeVisible();
  expect(screen.getByTitle("123.000000000001")).toBeVisible();
});
it("labels stale refreshes and clears account details after lost authorization", async () => {
  const client = mount();
  await screen.findByText("Issued by account");
  get.mockRejectedValue(new Error("offline"));
  await act(() => client.invalidateQueries());
  expect(
    await screen.findByText(
      "Showing previously loaded issuance totals. Refresh failed.",
    ),
  ).toBeVisible();
  get.mockRejectedValue(
    new ApiError(403, {
      error: "forbidden",
      error_code: 1003,
      message: "Forbidden",
    }),
  );
  await act(() => client.invalidateQueries());
  await waitFor(() =>
    expect(screen.queryByText("Issued by account")).not.toBeInTheDocument(),
  );
  expect(screen.queryByTitle("123.000000000001")).not.toBeInTheDocument();
});
it("renders the complete empty population without suggesting consumption or remaining balance", async () => {
  get.mockResolvedValue({
    ...response(),
    issued: "0",
    grants: 0,
    total_accounts: 0,
    accounts: [],
  });
  mount();
  await screen.findByText("Issued by account");
  await userEvent.click(screen.getByText("Issued by account"));
  expect(screen.getByText("No grants issued in this window.")).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Next accounts" }),
  ).not.toBeInTheDocument();
});
it("starts page one with the current relative window on explicit Refresh", async () => {
  get.mockImplementation((path: string) =>
    Promise.resolve(response(path.endsWith("page=2") ? 2 : 1)),
  );
  const client = new QueryClient();
  const ui = (refreshKey: number) => (
    <QueryClientProvider client={client}>
      <FreeGrantsReport filters={EMPTY_FILTERS} refreshKey={refreshKey} />
    </QueryClientProvider>
  );
  const view = render(ui(0));
  await screen.findByText("Issued by account");
  await userEvent.click(screen.getByText("Issued by account"));
  await userEvent.click(screen.getByRole("button", { name: "Next accounts" }));
  await screen.findByText("Research");
  view.rerender(ui(1));
  await waitFor(() =>
    expect(get.mock.calls.at(-1)![0]).toContain("period=7d&page=1"),
  );
  await userEvent.click(screen.getByText("Issued by account"));
  expect(screen.getByText("Ada")).toBeVisible();
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { AccountsSection } from "./accounts-section";
import { InventorySection } from "./inventory-section";

const mock = vi.hoisted(() => ({ get: vi.fn(), apiClient: vi.fn() }));
vi.mock("@/lib/api-client", () => ({
  api: { get: mock.get },
  apiClient: mock.apiClient,
  ApiError: class ApiError extends Error {},
}));

const ids = {
  admin: "3f0b8a3e-6c55-4b1e-9a61-1c2f0e9a7d10",
  line: "00000000-0000-4000-8000-000000000002",
  claim: "8f8a6e93-1f0c-4f3e-9d77-2a8b9b0b6c3a",
  period: "a7c0f5d2-5b8e-4d3a-8c5f-0e1d2c3b4a59",
  account: "c3d2e1f0-9a8b-4c7d-8e6f-5a4b3c2d1e0f",
  cursor: "0b8f4b44-3d73-4d0c-8f59-5d1c4f7b2a61",
};
const MESSAGE =
  "These stored records could not be read and were skipped. Repair them so they appear here again:";

// The first page names one skipped row; the next page repeats it and adds two.
function pages(path: string, first: string, more: string[]) {
  if (path.startsWith("/admin/managed-comms/eligibility"))
    return { eligibility: [], next_after_id: null };
  const key = path.startsWith("/admin/managed-lines/accounts") ? "accounts" : "lines";
  return path.includes("?after=")
    ? { [key]: [], next_after_id: null, premium_enabled: false, unreadable_ids: [first, ...more] }
    : { [key]: [], next_after_id: ids.cursor, premium_enabled: false, unreadable_ids: [first] };
}
function view(node: ReactElement) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

beforeEach(() => {
  useAuthStore.getState().setUser({
    id: ids.admin,
    email: "admin@example.com",
    display_name: "Admin",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: true,
    is_active: true,
    created_at: "2026-11-01T00:00:00Z",
  });
  mock.get.mockImplementation(async (path: string) =>
    path.startsWith("/admin/managed-lines/accounts")
      ? pages(path, ids.account, [])
      : pages(path, ids.line, [ids.claim, ids.period]),
  );
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

it("names skipped line, claim and period records across loaded pages", async () => {
  view(<InventorySection />);
  const banner = await screen.findByRole("alert");
  expect(banner).toHaveTextContent(MESSAGE);
  expect(within(banner).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
    ids.line,
  ]);
  await userEvent.click(await screen.findByRole("button", { name: "Load more lines" }));
  await vi.waitFor(() =>
    expect(within(screen.getByRole("alert")).getAllByRole("listitem")).toHaveLength(3),
  );
  expect(
    within(screen.getByRole("alert"))
      .getAllByRole("listitem")
      .map((item) => item.textContent),
  ).toEqual([ids.line, ids.claim, ids.period]);
  expect(mock.get).toHaveBeenCalledWith(
    `/admin/managed-lines?after=${ids.cursor}`,
    expect.anything(),
  );
});

it("names skipped account records and shows nothing when every row was readable", async () => {
  view(<AccountsSection />);
  const banner = await screen.findByRole("alert");
  expect(banner).toHaveTextContent(MESSAGE);
  expect(within(banner).getByRole("listitem")).toHaveTextContent(ids.account);
  await userEvent.click(await screen.findByRole("button", { name: "Load more accounts" }));
  // Once the last page loaded, the ID it repeated is still listed once.
  await vi.waitFor(() =>
    expect(screen.queryByRole("button", { name: "Load more accounts" })).toBeNull(),
  );
  expect(mock.get).toHaveBeenCalledWith(
    `/admin/managed-lines/accounts?after=${ids.cursor}`,
    expect.anything(),
  );
  expect(within(screen.getByRole("alert")).getAllByRole("listitem")).toHaveLength(1);
  cleanup();
  mock.get.mockImplementation(async (path: string) =>
    path.startsWith("/admin/managed-comms/eligibility")
      ? { eligibility: [], next_after_id: null }
      : { accounts: [], next_after_id: null, premium_enabled: false },
  );
  view(<AccountsSection />);
  await screen.findByText("Managed line accounts");
  expect(screen.queryByText(MESSAGE)).toBeNull();
});

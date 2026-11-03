import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { NyxbotRealWork } from "./nyxbot-real-work";
import { api } from "@/lib/api-client";
import type { WeeklyRealWork } from "@/schemas/assistant-real-work";
const state = vi.hoisted(() => ({
  user: { id: "viewer", is_admin: false, is_operator: false },
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (s: typeof state) => unknown) => select(state),
}));
vi.mock("@/lib/api-client", () => ({ api: { get: vi.fn(), put: vi.fn() } }));
const owner = "11111111-1111-4111-8111-111111111111";
const data: WeeklyRealWork = {
  threshold: 3,
  timezone: "Asia/Singapore",
  weeks: [
    {
      week: "2026-11-02",
      partial: true,
      merchants: 2,
      internal_owners: 1,
      completed: 6,
      attempted: 8,
      triggered: 3,
      helpful: 3,
      votes: 4,
      internal_completed: 3,
      internal_attempted: 3,
      internal_triggered: 1,
      internal_helpful: 1,
      internal_votes: 1,
      owners: [
        {
          owner_id: owner,
          internal: false,
          completed: 3,
          attempted: 4,
          triggered: 2,
          helpful: 1,
          votes: 1,
        },
      ],
      owners_truncated: false,
      funnel: [
        {
          internal: false,
          owners: 8,
          has_nyxbot: 6,
          connected_business: 4,
          assigned_channel_work: 3,
          first_real_work: 2,
          median_seconds_to_first_work: 7200,
        },
      ],
    },
  ],
};
function view(value = data) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <NyxbotRealWork data={value} />
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  vi.clearAllMocks();
  state.user.is_admin = false;
  state.user.is_operator = false;
  vi.mocked(api.get).mockResolvedValue({ owner_ids: [owner] });
});
it("shows Singapore weeks, merchant and internal counts, trigger share and restricted rates", () => {
  view();
  const table = within(
    screen.getByRole("table", { name: "Weekly real work totals" }),
  );
  expect(table.getByText("Merchants ≥ 3")).toBeInTheDocument();
  expect(table.getByText("Internal ≥ 3")).toBeInTheDocument();
  expect(table.getByText("Partial")).toBeInTheDocument();
  expect(table.getByText("50%")).toBeInTheDocument();
  expect(table.getAllByText("75%")).toHaveLength(2);
  expect(
    screen.getByText(/Monday–Sunday · Asia\/Singapore/),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByText("2026-11-02 · Owners and activation"));
  expect(screen.getByText(owner)).toBeInTheDocument();
  expect(
    screen.getByRole("table", { name: "Activation funnel 2026-11-02" }),
  ).toHaveTextContent("2.0 hr");
});
it("shows empty and bounded owner detail without inventing rates", () => {
  view({
    ...data,
    threshold: 7,
    weeks: [
      {
        ...data.weeks[0]!,
        completed: 0,
        attempted: 0,
        votes: 0,
        owners: [],
        owners_truncated: true,
        funnel: [],
      },
    ],
  });
  expect(screen.getByText("Merchants ≥ 7")).toBeInTheDocument();
  fireEvent.click(screen.getByText("2026-11-02 · Owners and activation"));
  expect(
    screen.getByText("No real work recorded in this week."),
  ).toBeInTheDocument();
  expect(screen.getByText(/Showing the top 100 owners/)).toBeInTheDocument();
});
it("lets a usage viewer read the list without offering a write", async () => {
  view();
  fireEvent.click(screen.getByRole("button", { name: "Internal owners" }));
  const dialog = within(await screen.findByRole("dialog"));
  await waitFor(() => expect(dialog.getByText(owner)).toBeInTheDocument());
  expect(
    dialog.queryByRole("button", { name: "Save internal owners" }),
  ).not.toBeInTheDocument();
  expect(api.put).not.toHaveBeenCalled();
});
it("lets operators edit owner UUIDs and leaves errors visible", async () => {
  state.user.is_operator = true;
  vi.mocked(api.put).mockRejectedValueOnce(new Error("unavailable"));
  view();
  fireEvent.click(screen.getByRole("button", { name: "Internal owners" }));
  const input = await screen.findByLabelText("Person or organization IDs");
  fireEvent.change(input, {
    target: { value: "22222222-2222-4222-8222-222222222222" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save internal owners" }));
  await screen.findByText(/Could not save/);
  expect(api.put).toHaveBeenCalledWith(
    "/admin/assistant/analytics/internal-owners",
    { owner_ids: ["22222222-2222-4222-8222-222222222222"] },
  );
  expect(screen.getByRole("dialog")).toBeInTheDocument();
});

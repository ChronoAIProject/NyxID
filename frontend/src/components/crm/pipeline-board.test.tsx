import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import { CrmPipelineBoard } from "@/components/crm/pipeline-board";
import type { CrmTableQuery } from "@/schemas/crm";

const { aggregate, table, move, retryCounts } = vi.hoisted(() => ({
  aggregate: vi.fn(),
  table: vi.fn(),
  move: vi.fn(),
  retryCounts: vi.fn(),
}));
const object = {
  owner_id: "org-a",
  key: "opportunity",
  access: "work",
  pipeline: { stage_field: "stage" },
  fields: [
    {
      key: "name",
      label: "Name",
      type: "text",
      required: true,
      archived: false,
    },
    {
      key: "stage",
      label: "Stage",
      type: "select",
      required: true,
      archived: false,
      options: [
        { value: "new", label: "New" },
        { value: "won", label: "Won" },
      ],
    },
  ],
};
const record = {
  id: "deal-a",
  owner_id: "org-a",
  object_key: "opportunity",
  revision: 3,
  values: { name: "First deal", stage: "new" },
  links: {},
  overrides: [],
  archived: false,
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
};
vi.mock("@/hooks/use-crm", () => ({
  useCrmObject: () => ({ data: object, isPending: false, isError: false }),
  useCrmAggregate: aggregate,
  useCrmTableQuery: table,
  useMoveCrmRecord: () => ({ mutateAsync: move, isPending: false }),
}));
const query: CrmTableQuery = {
  object: "opportunity",
  filters: [],
  select: ["stage"],
  page_size: 50,
  include_archived: false,
};
function setup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const invalidate = vi.spyOn(client, "invalidateQueries");
  render(
    <QueryClientProvider client={client}>
      <CrmPipelineBoard query={query} ownerId="org-a" />
    </QueryClientProvider>,
  );
  return { invalidate };
}
beforeEach(() => {
  vi.resetAllMocks();
  aggregate.mockReturnValue({
    data: {
      points: [
        { key: "new", value: 75 },
        { key: "won", value: 2 },
      ],
      total: 77,
    },
    isPending: false,
    isError: false,
    refetch: retryCounts,
  });
  table.mockImplementation((q: CrmTableQuery) => ({
    data: {
      object,
      rows: q.filters.some(
        (filter) =>
          filter.value === "won" ||
          (Array.isArray(filter.value) && filter.value.includes("won")),
      )
        ? []
        : q.cursor
          ? [
              {
                ...record,
                id: "deal-b",
                values: { name: "Later deal", stage: "new" },
              },
            ]
          : [record],
      next_cursor: q.cursor ? null : "page-2",
      total_count: 75,
    },
    isPending: false,
    isError: false,
    isFetching: false,
    refetch: vi.fn(),
  }));
  move.mockResolvedValue({
    ...record,
    revision: 4,
    values: { ...record.values, stage: "won" },
  });
});
it("pages lane cards while retaining authoritative counts and populated labels in narrow projections", () => {
  setup();
  const lane = screen.getByRole("region", { name: "New" });
  expect(within(lane).getByText("75")).toBeInTheDocument();
  expect(within(lane).getByText("First deal")).toBeInTheDocument();
  fireEvent.click(within(lane).getByText("Next cards"));
  expect(within(lane).getByText("Later deal")).toBeInTheDocument();
  expect(within(lane).getByText("75")).toBeInTheDocument();
  expect(table).toHaveBeenCalledWith(
    expect.objectContaining({
      select: expect.arrayContaining(["name", "stage"]),
      cursor: "page-2",
      include_archived: false,
    }),
    "org-a",
    expect.anything(),
  );
  fireEvent.click(within(lane).getByText("Previous cards"));
  expect(within(lane).getByText("First deal")).toBeInTheDocument();
});
it("shows count failure and recovery without manufacturing zero lane totals", () => {
  aggregate.mockReturnValue({
    data: undefined,
    isPending: false,
    isError: true,
    refetch: retryCounts,
  });
  setup();
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Pipeline counts are unavailable",
  );
  expect(
    within(screen.getByRole("region", { name: "New" })).getByText(
      "Unavailable",
    ),
  ).toBeInTheDocument();
  expect(screen.queryByText("0")).not.toBeInTheDocument();
  fireEvent.click(screen.getByText("Retry counts"));
  expect(retryCounts).toHaveBeenCalledOnce();
});
it("reconciles an uncertain move and retries the identical logical request", async () => {
  move.mockRejectedValueOnce(new Error("Connection lost"));
  const { invalidate } = setup();
  const user = userEvent.setup();
  await user.click(
    screen.getByRole("button", { name: "Move First deal to another stage" }),
  );
  await user.click(
    await screen.findByRole("menuitem", { name: "Move to Won" }),
  );
  await screen.findByText("Retry same move");
  expect(invalidate).toHaveBeenCalledWith({ queryKey: ["crm", "org-a"] });
  const original = move.mock.calls[0]![0];
  expect(original).toMatchObject({
    id: "deal-a",
    expectedRevision: 3,
    to: "won",
    idempotencyKey: expect.any(String),
  });
  fireEvent.click(screen.getByText("Retry same move"));
  await waitFor(() => expect(move).toHaveBeenCalledTimes(2));
  expect(move.mock.calls[1]![0]).toEqual(original);
  await waitFor(() =>
    expect(screen.queryByText("Retry same move")).not.toBeInTheDocument(),
  );
});

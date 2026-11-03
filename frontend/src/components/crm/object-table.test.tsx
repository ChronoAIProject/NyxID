import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { CrmObject, CrmTableQuery } from "@/schemas/crm";
import { CrmObjectTable } from "./object-table";

const mocks = vi.hoisted(() => ({
  queries: [] as CrmTableQuery[],
  tableError: false,
}));

const object: CrmObject = {
  id: "company-object",
  owner_id: "user-1",
  key: "company",
  label: "Company",
  plural_label: "Companies",
  schema_revision: 1,
  fields: [
    {
      key: "name",
      label: "Name",
      required: true,
      archived: false,
      type: "text",
      max_length: 1000,
    },
  ],
  pipeline: null,
  default_columns: ["name"],
  seeded: true,
  access: "work",
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
};

vi.mock("@/hooks/use-crm", () => ({
  useCrmObject: () => ({
    data: object,
    isPending: false,
    isError: false,
    refetch: vi.fn(),
  }),
  useCrmTableQuery: (query: CrmTableQuery) => {
    mocks.queries.push(query);
    return mocks.tableError
      ? {
          data: undefined,
          isPending: false,
          isFetching: false,
          isError: true,
          refetch: vi.fn(),
        }
      : {
          data: {
            object,
            rows: [],
            next_cursor: null,
            total_count: 0,
          },
          isPending: false,
          isFetching: false,
          isError: false,
          refetch: vi.fn(),
        };
  },
}));

beforeEach(() => {
  mocks.queries = [];
  mocks.tableError = false;
});

afterEach(cleanup);

it("keeps an incomplete filter draft out of the executed query", async () => {
  const user = userEvent.setup();
  render(<CrmObjectTable query={{ object: "company", filters: [] }} />);

  await user.click(screen.getByRole("button", { name: "Filters" }));
  await user.click(screen.getByRole("combobox", { name: "Add filter" }));
  await user.click(screen.getByRole("option", { name: "Name" }));

  expect(screen.getByLabelText("Name value")).toBeInTheDocument();
  expect(mocks.queries.at(-1)?.filters).toEqual([]);
});

it("keeps filter controls mounted when the query fails", async () => {
  mocks.tableError = true;
  const user = userEvent.setup();
  render(
    <CrmObjectTable
      query={{
        object: "company",
        filters: [{ field: "name", op: "contains", value: "north" }],
      }}
    />,
  );

  expect(screen.getByRole("alert")).toHaveTextContent(
    "Could not load records.",
  );
  expect(screen.getByDisplayValue("north")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Remove Name filter" }));
  expect(mocks.queries.at(-1)?.filters).toEqual([]);
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Could not load records.",
  );
});

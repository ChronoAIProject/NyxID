import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import type { CrmObject, CrmRecord } from "@/schemas/crm";
import { CrmRecordSheet } from "./record-sheet";

const mocks = vi.hoisted(() => ({
  update: vi.fn(),
  refetch: vi.fn(),
}));

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}));
vi.mock("@/components/crm/activity-timeline", () => ({
  CrmActivityTimeline: () => null,
}));
vi.mock("@/hooks/use-crm", () => ({
  useCrmRecord: () => ({
    data: latestRecord,
    isPending: false,
    isError: false,
    refetch: mocks.refetch,
  }),
  useCrmObject: () => ({
    data: object,
    isPending: false,
    isError: false,
  }),
  useCrmRelated: () => ({
    data: { groups: [] },
    isPending: false,
  }),
  useSetCrmRecordArchived: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
  useUpdateCrmRecord: () => ({
    mutateAsync: mocks.update,
    isPending: false,
  }),
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
    {
      key: "meeting",
      label: "Meeting",
      required: false,
      archived: false,
      type: "datetime",
    },
    {
      key: "active",
      label: "Active",
      required: false,
      archived: false,
      type: "boolean",
    },
  ],
  pipeline: null,
  default_columns: ["name", "meeting"],
  seeded: true,
  access: "work",
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
};

const record: CrmRecord = {
  id: "company-1",
  owner_id: "user-1",
  object_key: "company",
  revision: 1,
  values: {
    name: "Northwind",
    meeting: "2026-11-04T12:34:56.789Z",
  },
  links: {},
  overrides: [],
  archived: false,
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
};

let latestRecord = record;

beforeEach(() => {
  latestRecord = record;
  mocks.update.mockReset();
  mocks.refetch.mockReset();
});

afterEach(cleanup);

it("retries a revision conflict with only the human's field delta", async () => {
  const current: CrmRecord = {
    ...record,
    revision: 2,
    values: {
      ...record.values,
      meeting: "2026-11-04T13:45:12.345Z",
    },
    updated_at: "2026-11-04T00:01:00Z",
  };
  mocks.update
    .mockRejectedValueOnce(
      new ApiError(409, {
        error: "crm_revision_conflict",
        error_code: 13400,
        message: "The record changed",
        current,
      }),
    )
    .mockResolvedValueOnce({
      ...current,
      values: { ...current.values, name: "Contoso" },
    });
  const user = userEvent.setup();

  render(<CrmRecordSheet recordId={record.id} open onOpenChange={vi.fn()} />);
  await user.clear(screen.getByLabelText("Name *"));
  await user.type(screen.getByLabelText("Name *"), "Contoso");
  await user.click(screen.getByRole("button", { name: "Save changes" }));

  expect(
    await screen.findByText("This record changed elsewhere"),
  ).toBeInTheDocument();
  expect(mocks.update.mock.calls[0]?.[0]).toMatchObject({
    expectedRevision: 1,
    values: { name: "Contoso" },
    protect: [],
    unprotect: [],
  });
  expect(mocks.update.mock.calls[0]?.[0].values).not.toHaveProperty("meeting");
  expect(mocks.update.mock.calls[0]?.[0].values).not.toHaveProperty("active");

  await user.click(screen.getByRole("button", { name: "Retry my changes" }));
  expect(mocks.update).toHaveBeenCalledTimes(2);
  expect(mocks.update.mock.calls[1]?.[0]).toMatchObject({
    expectedRevision: 2,
    values: { name: "Contoso" },
    protect: [],
    unprotect: [],
  });
  expect(mocks.update.mock.calls[1]?.[0].idempotencyKey).not.toBe(
    mocks.update.mock.calls[0]?.[0].idempotencyKey,
  );
});

it("keeps a dirty draft across a newer live revision and resets on equal-revision identity switch", async () => {
  const user = userEvent.setup();
  const view = render(
    <CrmRecordSheet recordId={record.id} open onOpenChange={vi.fn()} />,
  );
  const name = screen.getByRole("textbox", { name: /^Name/ });
  await user.clear(name);
  await user.type(name, "Unsaved company");
  latestRecord = {
    ...record,
    revision: record.revision + 1,
    values: { ...record.values, name: "Server update" },
  };
  view.rerender(
    <CrmRecordSheet recordId={record.id} open onOpenChange={vi.fn()} />,
  );
  expect(screen.getByRole("textbox", { name: /^Name/ })).toHaveValue(
    "Unsaved company",
  );
  latestRecord = {
    ...latestRecord,
    id: "company-2",
    values: { ...latestRecord.values, name: "Other company" },
  };
  view.rerender(
    <CrmRecordSheet recordId="company-2" open onOpenChange={vi.fn()} />,
  );
  expect(screen.getByRole("textbox", { name: /^Name/ })).toHaveValue(
    "Other company",
  );
});

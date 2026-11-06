import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CrmRecordForm } from "./record-form";
import type { CrmObject, CrmRecord } from "@/schemas/crm";

const mutateAsync = vi.fn();
vi.mock("@/hooks/use-crm", () => ({
  useCreateCrmRecord: () => ({ mutateAsync, isPending: false }),
  useCrmObject: () => ({ data: undefined, isPending: false }),
  useCrmTableQuery: () => ({ data: undefined, isPending: false }),
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
      key: "domain",
      label: "Domain",
      required: false,
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

const created = {
  id: "company-1",
  owner_id: "user-1",
  object_key: "company",
  revision: 1,
  values: { name: "Northwind", domain: "northwind.test" },
  links: {},
  overrides: [],
  archived: false,
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
} satisfies CrmRecord;

describe("CrmRecordForm", () => {
  beforeEach(() => mutateAsync.mockReset());

  it("always renders required fields and reuses one key when the same submit is retried", async () => {
    mutateAsync.mockRejectedValueOnce(new Error("Connection interrupted"));
    mutateAsync.mockResolvedValueOnce(created);
    const user = userEvent.setup();
    render(
      <CrmRecordForm object={object} fields={["domain"]} title="Add company" />,
    );

    await user.type(screen.getByLabelText("Name *"), "Northwind");
    await user.type(screen.getByLabelText("Domain"), "northwind.test");
    await user.click(screen.getByRole("button", { name: "Create record" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Connection interrupted",
    );
    await user.click(screen.getByRole("button", { name: "Create record" }));

    expect(mutateAsync).toHaveBeenCalledTimes(2);
    expect(mutateAsync.mock.calls[0]?.[0].values).toEqual({
      name: "Northwind",
      domain: "northwind.test",
    });
    expect(mutateAsync.mock.calls[1]?.[0].idempotencyKey).toBe(
      mutateAsync.mock.calls[0]?.[0].idempotencyKey,
    );
  });
});

it("keeps label, error and heading associations unique across two forms for one object", () => {
  const linked: CrmObject = {
    ...object,
    fields: [
      ...object.fields,
      {
        key: "company",
        label: "Company",
        type: "record",
        target: "company",
        cardinality: "one",
        required: false,
        archived: false,
      },
    ],
  };
  const { container } = render(
    <>
      <CrmRecordForm object={linked} title="First company" />
      <CrmRecordForm object={linked} title="Second company" />
    </>,
  );
  const ids = [...container.querySelectorAll("[id]")].map(
    (element) => element.id,
  );
  expect(new Set(ids).size).toBe(ids.length);
  for (const label of container.querySelectorAll("label[for]"))
    expect(
      container.querySelector(`[id="${label.getAttribute("for")}"]`),
    ).toBeTruthy();
  expect(screen.getAllByRole("combobox", { name: "Company" })).toHaveLength(2);
  expect(screen.getByRole("region", { name: "First company" })).not.toBe(
    screen.getByRole("region", { name: "Second company" }),
  );
});

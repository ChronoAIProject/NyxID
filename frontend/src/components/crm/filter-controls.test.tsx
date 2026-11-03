import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { CrmFilterControls } from "./filter-controls";
import type { CrmObject } from "@/schemas/crm";

vi.mock("@/components/crm/record-picker", () => ({
  CrmRecordPicker: () => <div>Record picker</div>,
}));

const object: CrmObject = {
  id: "contact-object",
  owner_id: "user-1",
  key: "contact",
  label: "Contact",
  plural_label: "Contacts",
  schema_revision: 1,
  fields: [
    {
      key: "company",
      label: "Company",
      required: false,
      archived: false,
      type: "record",
      target: "company",
      cardinality: "one",
    },
  ],
  pipeline: null,
  default_columns: ["company"],
  seeded: true,
  access: "work",
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
};

describe("CrmFilterControls", () => {
  it("offers only operators legal for the selected field kind", async () => {
    const user = userEvent.setup();
    render(
      <CrmFilterControls
        object={object}
        filters={[{ field: "company", op: "eq", value: null }]}
        onChange={vi.fn()}
      />,
    );
    await user.click(
      screen.getByRole("combobox", { name: "Filter 1 operator" }),
    );
    expect(screen.getByRole("option", { name: "is" })).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: "is any of" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "is set" })).toBeInTheDocument();
    expect(
      screen.queryByRole("option", { name: "contains" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("option", { name: "is not" }),
    ).not.toBeInTheDocument();
  });
});

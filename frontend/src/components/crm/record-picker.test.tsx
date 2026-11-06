import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { CrmRecordPicker } from "./record-picker";
vi.mock("@/hooks/use-crm", () => ({
  useCrmObject: (_key: string, _owner: string, enabled: boolean) => ({
    data: enabled
      ? {
          label: "Company",
          plural_label: "Companies",
          fields: [{ key: "name", required: true, type: "text" }],
        }
      : undefined,
  }),
  useCrmTableQuery: (query: unknown) => ({
    data: query
      ? {
          rows: [
            {
              id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
              values: { name: "Northwind" },
            },
          ],
          next_cursor: null,
        }
      : undefined,
  }),
}));
it("retains the selected label after the create-form picker closes and its query is disabled", async () => {
  function Picker() {
    const [value, setValue] = useState<string | readonly string[] | null>(null);
    return (
      <CrmRecordPicker
        field={{
          key: "company",
          label: "Company",
          type: "record",
          target: "company",
          cardinality: "one",
          required: false,
          archived: false,
        }}
        value={value}
        onChange={setValue}
      />
    );
  }
  render(<Picker />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("combobox", { name: "Company" }));
  await user.click(screen.getByRole("option", { name: "Northwind" }));
  expect(screen.getByRole("combobox", { name: "Company" })).toHaveTextContent(
    "Northwind",
  );
  expect(
    screen.getByRole("combobox", { name: "Company" }),
  ).not.toHaveTextContent("aaaaaaaa-");
});

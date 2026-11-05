import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { CrmSchemaEditor } from "./schema-editor";
import type { CrmObject } from "@/schemas/crm";
const { patch, add } = vi.hoisted(() => ({ patch: vi.fn(), add: vi.fn() }));
vi.mock("@/hooks/use-crm", () => ({
  useCreateCrmObject: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useUpdateCrmField: () => ({ mutateAsync: patch, isPending: false }),
  useAddCrmField: () => ({ mutateAsync: add, isPending: false }),
}));
const object: CrmObject = {
  id: "schema",
  owner_id: "owner",
  key: "company",
  label: "Company",
  plural_label: "Companies",
  schema_revision: 1,
  fields: [
    {
      key: "name",
      label: "Name",
      type: "text",
      max_length: 1000,
      required: true,
      archived: false,
    },
    {
      key: "domain",
      label: "Domain",
      type: "text",
      max_length: 1000,
      required: false,
      archived: false,
    },
  ],
  default_columns: ["name"],
  pipeline: null,
  seeded: true,
  access: "schema",
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
};
beforeEach(() => {
  patch.mockReset();
  add.mockReset();
});
function save(field: string) {
  const input = screen.getByLabelText(`${field} label`);
  const form = input.closest("form")!;
  fireEvent.click(within(form).getByRole("button", { name: "Save field" }));
}
it("advances pristine fields after one save and retains dirty concurrent baselines until reload", async () => {
  let current = object;
  const view = render(<CrmSchemaEditor object={current} />);
  patch.mockImplementation(async ({ fieldKey, change }) => {
    current = {
      ...current,
      schema_revision: current.schema_revision + 1,
      fields: current.fields.map((f) =>
        f.key === fieldKey ? { ...f, ...change } : f,
      ),
    };
    return current;
  });
  fireEvent.change(screen.getByLabelText("name label"), {
    target: { value: "Company name" },
  });
  save("name");
  await waitFor(() => expect(patch).toHaveBeenCalledTimes(1));
  view.rerender(<CrmSchemaEditor object={current} />);
  fireEvent.change(screen.getByLabelText("domain label"), {
    target: { value: "Website domain" },
  });
  save("domain");
  await waitFor(() => expect(patch).toHaveBeenCalledTimes(2));
  expect(patch.mock.calls.map(([arg]) => arg.expectedSchemaRevision)).toEqual([
    1, 2,
  ]);
  view.rerender(<CrmSchemaEditor object={current} />);
  fireEvent.change(screen.getByLabelText("name label"), {
    target: { value: "Dirty label" },
  });
  current = { ...current, schema_revision: 4 };
  view.rerender(<CrmSchemaEditor object={current} />);
  expect(screen.getByLabelText("name label")).toHaveValue("Dirty label");
  expect(
    screen.getByText(/Schema changed while you were editing/),
  ).toBeVisible();
  fireEvent.click(screen.getByText("Reload name"));
  expect(screen.getByLabelText("name label")).toHaveValue("Company name");
});
it("keeps an unchanged failed schema write identity and renews it after a payload edit", async () => {
  patch.mockRejectedValue(new Error("Network"));
  render(<CrmSchemaEditor object={object} />);
  fireEvent.change(screen.getByLabelText("domain label"), {
    target: { value: "New domain" },
  });
  save("domain");
  await screen.findByText("Network");
  save("domain");
  await waitFor(() => expect(patch).toHaveBeenCalledTimes(2));
  expect(patch.mock.calls[1]![0].idempotencyKey).toBe(
    patch.mock.calls[0]![0].idempotencyKey,
  );
  fireEvent.change(screen.getByLabelText("domain label"), {
    target: { value: "Another domain" },
  });
  save("domain");
  await waitFor(() => expect(patch).toHaveBeenCalledTimes(3));
  expect(patch.mock.calls[2]![0].idempotencyKey).not.toBe(
    patch.mock.calls[1]![0].idempotencyKey,
  );
});

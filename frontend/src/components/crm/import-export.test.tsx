import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { CrmImportResult } from "@/hooks/use-crm";
import type { CrmObject } from "@/schemas/crm";
import { CrmImportExport } from "./import-export";

const { importRows } = vi.hoisted(() => ({ importRows: vi.fn() }));
vi.mock("@/hooks/use-crm", () => ({
  useCrmImport: () => ({ mutateAsync: importRows, isPending: false }),
  useCrmExport: () => ({ isPending: false }),
}));

const company: CrmObject = {
  id: "company-object",
  owner_id: "person-1",
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
      max_length: 1_000,
    },
  ],
  pipeline: null,
  default_columns: ["name"],
  seeded: true,
  access: "work",
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
});

it.each([
  [1, "1 row · 1 create · 0 update · 0 skip"],
  [2, "2 rows · 2 create · 0 update · 0 skip"],
] as const)(
  "shows the row count for a %i-row import preview",
  async (count, expected) => {
    const rows = Array.from({ length: count }, (_, index) => ({
      index,
      matched_id: null,
      revision: null,
      action: "create",
      errors: [],
    }));
    const result: CrmImportResult = {
      committed: false,
      snapshot: { schema_revision: 1, payload_digest: "digest", rows },
      rows,
    };
    importRows.mockResolvedValue(result);
    render(
      <CrmImportExport
        object={company}
        query={{ object: "company", filters: [] }}
      />,
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Import source" }), {
      target: {
        value: JSON.stringify(
          Array.from({ length: count }, (_, index) => ({
            name: `Company ${index}`,
          })),
        ),
      },
    });
    fireEvent.click(screen.getByRole("button", { name: "Map columns" }));
    fireEvent.click(screen.getByRole("button", { name: "Preview import" }));
    expect(await screen.findByText(expected)).toBeInTheDocument();
  },
);

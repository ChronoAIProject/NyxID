import { describe, expect, it } from "vitest";
import {
  crmFilterOperators,
  formatCrmFieldInput,
  isCrmFilterOperatorAllowed,
  parseCrmFieldInput,
} from "./field-kinds";
import type { CrmField } from "@/schemas/crm";

const base = {
  label: "Value",
  required: false,
  archived: false,
} as const;

describe("CRM field-kind registry", () => {
  it("round-trips currency as integer minor units", () => {
    const field: CrmField = {
      ...base,
      key: "amount",
      type: "currency",
      currency: "USD",
    };
    expect(parseCrmFieldInput(field, "1250.00")).toBe(125000);
    expect(formatCrmFieldInput(field, 125000)).toBe("1250.00");
    expect(
      formatCrmFieldInput(field, { amount_minor: -305, currency: "USD" }),
    ).toBe("-3.05");
    expect(() => parseCrmFieldInput(field, "12.345")).toThrow(
      "at most 2 decimal places",
    );
  });

  it("round-trips percentages through their stored fraction", () => {
    const field: CrmField = {
      ...base,
      key: "probability",
      type: "number",
      format: "percent",
    };
    expect(parseCrmFieldInput(field, "25")).toBe(0.25);
    expect(formatCrmFieldInput(field, 0.25)).toBe("25");
  });

  it("keeps calendar dates byte-identical without timezone conversion", () => {
    const field: CrmField = { ...base, key: "due", type: "date" };
    expect(parseCrmFieldInput(field, "2026-03-29")).toBe("2026-03-29");
    expect(formatCrmFieldInput(field, "2026-03-29")).toBe("2026-03-29");
    expect(() => parseCrmFieldInput(field, "2026-02-30")).toThrow(
      "valid calendar date",
    );
  });

  it("exposes only the contract operators for each kind", () => {
    const text: CrmField = {
      ...base,
      key: "name",
      type: "text",
      max_length: 1000,
    };
    const record: CrmField = {
      ...base,
      key: "company",
      type: "record",
      target: "company",
      cardinality: "one",
    };
    expect(crmFilterOperators(text)).toEqual([
      "eq",
      "neq",
      "in",
      "contains",
      "is_set",
      "is_unset",
    ]);
    expect(crmFilterOperators(record)).toEqual([
      "eq",
      "in",
      "is_set",
      "is_unset",
    ]);
    expect(isCrmFilterOperatorAllowed(record, "contains")).toBe(false);
  });
});

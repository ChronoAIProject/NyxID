import { describe, expect, it } from "vitest";
import {
  crmPipelineCountQuery,
  crmPipelineLaneQuery,
  crmPipelineStageMatches,
} from "./pipeline-query";
import type { CrmTableQuery } from "@/schemas/crm";

describe("CRM pipeline query refinement", () => {
  it("folds a saved stage predicate into a 20-filter lane query", () => {
    const query: CrmTableQuery = {
      object: "opportunity",
      filters: [
        { field: "field_01", op: "eq", value: "value-01" },
        { field: "field_02", op: "eq", value: "value-02" },
        { field: "field_03", op: "eq", value: "value-03" },
        { field: "field_04", op: "eq", value: "value-04" },
        { field: "field_05", op: "eq", value: "value-05" },
        { field: "field_06", op: "eq", value: "value-06" },
        { field: "field_07", op: "eq", value: "value-07" },
        { field: "field_08", op: "eq", value: "value-08" },
        { field: "field_09", op: "eq", value: "value-09" },
        { field: "field_10", op: "eq", value: "value-10" },
        { field: "field_11", op: "eq", value: "value-11" },
        { field: "field_12", op: "eq", value: "value-12" },
        { field: "field_13", op: "eq", value: "value-13" },
        { field: "field_14", op: "eq", value: "value-14" },
        { field: "field_15", op: "eq", value: "value-15" },
        { field: "field_16", op: "eq", value: "value-16" },
        { field: "field_17", op: "eq", value: "value-17" },
        { field: "field_18", op: "eq", value: "value-18" },
        { field: "field_19", op: "eq", value: "value-19" },
        { field: "stage", op: "neq", value: "won" },
      ],
      page_size: 100,
    };

    expect(crmPipelineLaneQuery(query, "stage", ["new"])).toEqual({
      kind: "query",
      query: {
        object: "opportunity",
        filters: [
          { field: "field_01", op: "eq", value: "value-01" },
          { field: "field_02", op: "eq", value: "value-02" },
          { field: "field_03", op: "eq", value: "value-03" },
          { field: "field_04", op: "eq", value: "value-04" },
          { field: "field_05", op: "eq", value: "value-05" },
          { field: "field_06", op: "eq", value: "value-06" },
          { field: "field_07", op: "eq", value: "value-07" },
          { field: "field_08", op: "eq", value: "value-08" },
          { field: "field_09", op: "eq", value: "value-09" },
          { field: "field_10", op: "eq", value: "value-10" },
          { field: "field_11", op: "eq", value: "value-11" },
          { field: "field_12", op: "eq", value: "value-12" },
          { field: "field_13", op: "eq", value: "value-13" },
          { field: "field_14", op: "eq", value: "value-14" },
          { field: "field_15", op: "eq", value: "value-15" },
          { field: "field_16", op: "eq", value: "value-16" },
          { field: "field_17", op: "eq", value: "value-17" },
          { field: "field_18", op: "eq", value: "value-18" },
          { field: "field_19", op: "eq", value: "value-19" },
          { field: "stage", op: "eq", value: "new" },
        ],
        page_size: 50,
        cursor: undefined,
      },
    });
  });

  it("returns an empty lane when saved and lane stages do not intersect", () => {
    const query: CrmTableQuery = {
      object: "opportunity",
      filters: [{ field: "stage", op: "eq", value: "new" }],
    };

    expect(crmPipelineLaneQuery(query, "stage", ["qualified"])).toEqual({
      kind: "empty",
    });
  });

  it("keeps the original filters for authoritative counts", () => {
    const query: CrmTableQuery = {
      object: "opportunity",
      filters: [
        { field: "stage", op: "eq", value: "new" },
        { field: "amount", op: "gte", value: 10000 },
      ],
    };

    expect(crmPipelineCountQuery(query, "stage").filters).toEqual(
      query.filters,
    );
  });

  it("applies saved stage predicates to optimistic destinations", () => {
    const filters = [{ field: "stage", op: "neq", value: "won" }] as const;

    expect(crmPipelineStageMatches(filters, "stage", "qualified")).toBe(true);
    expect(crmPipelineStageMatches(filters, "stage", "won")).toBe(false);
  });
});

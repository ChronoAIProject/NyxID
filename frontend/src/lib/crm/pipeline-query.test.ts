import { describe, expect, it } from "vitest";
import {
  crmPipelineCountQuery,
  crmPipelineLaneQuery,
} from "./pipeline-query";
import type { CrmTableQuery } from "@/schemas/crm";

describe("CRM pipeline query refinement", () => {
  it("keeps saved stage predicates in lane queries", () => {
    const query: CrmTableQuery = {
      object: "opportunity",
      filters: [{ field: "stage", op: "neq", value: "won" }],
      page_size: 100,
    };

    expect(crmPipelineLaneQuery(query, "stage", ["won"])).toMatchObject({
      page_size: 50,
      filters: [
        { field: "stage", op: "neq", value: "won" },
        { field: "stage", op: "eq", value: "won" },
      ],
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
});

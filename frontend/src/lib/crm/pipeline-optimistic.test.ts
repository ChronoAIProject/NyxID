import { describe, expect, it } from "vitest";
import { applyOptimisticPipelineRows } from "./pipeline-optimistic";
import type { CrmRecordRow } from "@/schemas/crm";

const deal: CrmRecordRow = {
  id: "deal-1",
  revision: 3,
  values: { name: "Northwind", stage: "qualified" },
  links: {},
  overrides: [],
  archived: false,
};

describe("CRM pipeline optimistic moves", () => {
  it("adds the card to its destination and restores the source on rollback", () => {
    const pending = {
      [deal.id]: { record: deal, from: "qualified", to: "proposal" },
    };
    expect(applyOptimisticPipelineRows([deal], "qualified", pending)).toEqual(
      [],
    );
    expect(applyOptimisticPipelineRows([], "proposal", pending)).toEqual([
      deal,
    ]);

    expect(applyOptimisticPipelineRows([deal], "qualified", {})).toEqual([
      deal,
    ]);
    expect(applyOptimisticPipelineRows([], "proposal", {})).toEqual([]);
  });
});

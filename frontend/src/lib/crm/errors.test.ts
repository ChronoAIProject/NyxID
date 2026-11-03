import { describe, expect, it } from "vitest";
import { ApiError } from "@/lib/api-client";
import { crmConflictRecord, crmValidationErrors } from "./errors";
import type { CrmRecord } from "@/schemas/crm";

const actor = {
  kind: "human" as const,
  user_id: "user-1",
  agent_id: null,
  api_key_id: null,
};
const current: CrmRecord = {
  id: "record-1",
  owner_id: "user-1",
  object_id: "object-1",
  object_key: "company",
  revision: 8,
  values: { name: "Current name" },
  links: {},
  overrides: [],
  archived: false,
  created_by: actor,
  updated_by: actor,
  archived_at: null,
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:01:00Z",
};

describe("CRM recoverable errors", () => {
  it("returns the current record from a revision conflict", () => {
    const error = new ApiError(409, {
      error: "crm_revision_conflict",
      error_code: 13400,
      message: "The record changed",
      details: { current },
    });
    expect(crmConflictRecord(error)?.revision).toBe(8);
    expect(crmConflictRecord(error)?.values.name).toBe("Current name");
  });

  it("maps validation paths back to individual editors", () => {
    const error = new ApiError(422, {
      error: "crm_validation_failed",
      error_code: 13401,
      message: "Fix the fields",
      details: {
        errors: [
          { path: "values.amount.currency", message: "Currency must be USD" },
          { path: "values.expected_close", message: "Invalid calendar date" },
        ],
      },
    });
    expect(crmValidationErrors(error)).toEqual({
      amount: "Currency must be USD",
      expected_close: "Invalid calendar date",
    });
  });
});

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { ApiError } from "@/lib/api-client";
import { crmConflictRecord, crmValidationErrors } from "@/lib/crm/errors";
import type { ApiErrorResponse } from "@/types/api";
import {
  crmActivitiesResponseSchema,
  crmActivitySchema,
  crmAggregateResultSchema,
  crmObjectSchema,
  crmObjectsResponseSchema,
  crmRecordResponseSchema,
  crmRelatedResponseSchema,
  crmTableResultSchema,
} from "./crm";

function body(name: string): unknown {
  return JSON.parse(
    readFileSync(
      resolve(__dirname, `../../../docs/fixtures/crm/${name}.json`),
      "utf8",
    ),
  ) as unknown;
}

const successFixtures = [
  ["objects", crmObjectsResponseSchema],
  ["object_company", crmObjectSchema],
  ["field_create", crmObjectSchema],
  ["record_company_create", crmRecordResponseSchema],
  ["record_contact_create", crmRecordResponseSchema],
  ["record_opportunity_create", crmRecordResponseSchema],
  ["record_patch", crmRecordResponseSchema],
  ["record_move", crmRecordResponseSchema],
  ["activities", crmActivitiesResponseSchema],
  ["activity_note_create", crmActivitySchema],
  ["related", crmRelatedResponseSchema],
  ["table_page_1", crmTableResultSchema],
  ["table_page_2", crmTableResultSchema],
  ["aggregate_boolean", crmAggregateResultSchema],
  ["aggregate_empty_average", crmAggregateResultSchema],
  ["aggregate_truncated", crmAggregateResultSchema],
] as const;

describe("CRM real-route fixtures", () => {
  it.each(successFixtures)(
    "parses %s with its production schema",
    (name, schema) => {
      expect(() => schema.parse(body(name))).not.toThrow();
    },
  );

  it("parses record_revision_conflict through the UI conflict path", () => {
    const response = body("record_revision_conflict") as ApiErrorResponse;
    const current = crmConflictRecord(new ApiError(409, response));

    expect(current).not.toBeNull();
    expect(current).toMatchObject({
      id: "00000000-0000-4000-8000-000000000006",
      revision: 2,
      object_key: "company",
    });
  });

  it("parses record_validation_error through the UI field-error path", () => {
    const response = body("record_validation_error") as ApiErrorResponse;

    expect(crmValidationErrors(new ApiError(422, response))).toEqual({
      email: "must be a string",
      name: "is required",
    });
  });

  it("preserves the server's blank required-text error for the field editor", () => {
    const response = body("record_blank_required_text") as ApiErrorResponse;

    expect(crmValidationErrors(new ApiError(422, response))).toEqual({
      name: "required text cannot be blank",
    });
  });
});

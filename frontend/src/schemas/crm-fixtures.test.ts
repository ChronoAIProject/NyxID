import { describe, expect, it } from "vitest";
import activitiesRaw from "../../../docs/fixtures/crm/activities.json?raw";
import activityNoteCreateRaw from "../../../docs/fixtures/crm/activity_note_create.json?raw";
import aggregateBooleanRaw from "../../../docs/fixtures/crm/aggregate_boolean.json?raw";
import aggregateEmptyAverageRaw from "../../../docs/fixtures/crm/aggregate_empty_average.json?raw";
import aggregateTruncatedRaw from "../../../docs/fixtures/crm/aggregate_truncated.json?raw";
import fieldCreateRaw from "../../../docs/fixtures/crm/field_create.json?raw";
import objectCompanyRaw from "../../../docs/fixtures/crm/object_company.json?raw";
import objectsRaw from "../../../docs/fixtures/crm/objects.json?raw";
import recordCompanyCreateRaw from "../../../docs/fixtures/crm/record_company_create.json?raw";
import recordContactCreateRaw from "../../../docs/fixtures/crm/record_contact_create.json?raw";
import recordMoveRaw from "../../../docs/fixtures/crm/record_move.json?raw";
import recordOpportunityCreateRaw from "../../../docs/fixtures/crm/record_opportunity_create.json?raw";
import recordPatchRaw from "../../../docs/fixtures/crm/record_patch.json?raw";
import recordRevisionConflictRaw from "../../../docs/fixtures/crm/record_revision_conflict.json?raw";
import recordValidationErrorRaw from "../../../docs/fixtures/crm/record_validation_error.json?raw";
import recordBlankRequiredTextRaw from "../../../docs/fixtures/crm/record_blank_required_text.json?raw";
import relatedRaw from "../../../docs/fixtures/crm/related.json?raw";
import tablePage1Raw from "../../../docs/fixtures/crm/table_page_1.json?raw";
import tablePage2Raw from "../../../docs/fixtures/crm/table_page_2.json?raw";
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

function body(raw: string): unknown {
  return JSON.parse(raw) as unknown;
}

const successFixtures = [
  ["objects", objectsRaw, crmObjectsResponseSchema],
  ["object_company", objectCompanyRaw, crmObjectSchema],
  ["field_create", fieldCreateRaw, crmObjectSchema],
  ["record_company_create", recordCompanyCreateRaw, crmRecordResponseSchema],
  ["record_contact_create", recordContactCreateRaw, crmRecordResponseSchema],
  [
    "record_opportunity_create",
    recordOpportunityCreateRaw,
    crmRecordResponseSchema,
  ],
  ["record_patch", recordPatchRaw, crmRecordResponseSchema],
  ["record_move", recordMoveRaw, crmRecordResponseSchema],
  ["activities", activitiesRaw, crmActivitiesResponseSchema],
  ["activity_note_create", activityNoteCreateRaw, crmActivitySchema],
  ["related", relatedRaw, crmRelatedResponseSchema],
  ["table_page_1", tablePage1Raw, crmTableResultSchema],
  ["table_page_2", tablePage2Raw, crmTableResultSchema],
  ["aggregate_boolean", aggregateBooleanRaw, crmAggregateResultSchema],
  [
    "aggregate_empty_average",
    aggregateEmptyAverageRaw,
    crmAggregateResultSchema,
  ],
  ["aggregate_truncated", aggregateTruncatedRaw, crmAggregateResultSchema],
] as const;

describe("CRM real-route fixtures", () => {
  it.each(successFixtures)(
    "parses %s with its production schema",
    (_, raw, schema) => {
      expect(() => schema.parse(body(raw))).not.toThrow();
    },
  );

  it("parses record_revision_conflict through the UI conflict path", () => {
    const response = body(recordRevisionConflictRaw) as ApiErrorResponse;
    const current = crmConflictRecord(new ApiError(409, response));

    expect(current).not.toBeNull();
    expect(current).toMatchObject({
      id: "00000000-0000-4000-8000-000000000006",
      revision: 2,
      object_key: "company",
    });
  });

  it("parses record_validation_error through the UI field-error path", () => {
    const response = body(recordValidationErrorRaw) as ApiErrorResponse;

    expect(crmValidationErrors(new ApiError(422, response))).toEqual({
      email: "must be a string",
      name: "is required",
    });
  });

  it("preserves the server's blank required-text error for the field editor", () => {
    const response = body(recordBlankRequiredTextRaw) as ApiErrorResponse;

    expect(crmValidationErrors(new ApiError(422, response))).toEqual({
      name: "required text cannot be blank",
    });
  });
});

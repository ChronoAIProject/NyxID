import { ApiError } from "@/lib/api-client";
import {
  crmRevisionConflictDetailsSchema,
  crmValidationDetailsSchema,
  type CrmRecord,
} from "@/schemas/crm";

export function crmConflictRecord(error: unknown): CrmRecord | null {
  if (!(error instanceof ApiError) || error.errorCode !== 13400) return null;
  const parsed = crmRevisionConflictDetailsSchema.safeParse(
    error.errorResponse.current !== undefined
      ? { current: error.errorResponse.current }
      : error.errorResponse.details,
  );
  return parsed.success ? parsed.data.current : null;
}

export function crmValidationErrors(
  error: unknown,
): Readonly<Record<string, string>> {
  if (!(error instanceof ApiError) || error.errorCode !== 13401) return {};
  const parsed = crmValidationDetailsSchema.safeParse(
    error.errorResponse.errors !== undefined
      ? { errors: error.errorResponse.errors }
      : error.errorResponse.details,
  );
  if (!parsed.success) return {};
  return Object.fromEntries(
    parsed.data.errors.map((issue) => {
      const path = issue.path
        .replace(/^\$?\.?values\.?/, "")
        .replace(/^\//, "")
        .split(/[./]/)[0];
      return [path || "root", issue.message];
    }),
  );
}

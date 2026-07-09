/**
 * Depth-first search for the first human-readable message in a nested
 * react-hook-form error object (e.g.
 * `errors.ws_frame_injections[2].trigger.json_field_equals.path`).
 * Skips `ref`/`type`/`types` so DOM references are never traversed.
 */
export function firstNestedErrorMessage(error: unknown): string | undefined {
  if (!error || typeof error !== "object") return undefined;
  const record = error as Record<string, unknown>;
  if (typeof record.message === "string" && record.message.length > 0) {
    return record.message;
  }
  for (const [key, value] of Object.entries(record)) {
    if (key === "ref" || key === "type" || key === "types") continue;
    const message = firstNestedErrorMessage(value);
    if (message) return message;
  }
  return undefined;
}

/**
 * Flatten a numeric-keyed react-hook-form array-field error into a
 * row-index → message map for editors that render per-row errors. Without
 * this, nested array errors have no render site and a blocked submit gives
 * zero feedback (the NyxID#356 P3 bug class).
 */
export function flattenRowErrors(error: unknown): Record<number, string> {
  const rows: Record<number, string> = {};
  if (!error || typeof error !== "object") return rows;
  for (const [key, value] of Object.entries(error)) {
    const index = Number(key);
    if (!Number.isInteger(index) || index < 0) continue;
    const message = firstNestedErrorMessage(value);
    if (message) rows[index] = message;
  }
  return rows;
}

/**
 * Map zod issues from a direct `safeParse` (draft-state editors that
 * validate outside react-hook-form) into the same row → field → message
 * shape as {@link flattenRowFieldErrors}, so both kinds of surfaces feed
 * identical per-row highlighting.
 */
export function zodIssuesToRowFieldErrors(
  issues: ReadonlyArray<{
    readonly path: ReadonlyArray<PropertyKey>;
    readonly message: string;
  }>,
): Record<number, Record<string, string>> {
  const rows: Record<number, Record<string, string>> = {};
  for (const issue of issues) {
    const [row, field] = issue.path;
    if (typeof row !== "number") continue;
    const key = typeof field === "string" ? field : "root";
    const bucket = (rows[row] ??= {});
    if (!bucket[key]) bucket[key] = issue.message;
  }
  return rows;
}

/**
 * Like {@link flattenRowErrors} but preserves which field within each row
 * failed (row index → field name → message), so editors can highlight the
 * exact offending input instead of only printing a row-level message.
 */
export function flattenRowFieldErrors(
  error: unknown,
): Record<number, Record<string, string>> {
  const rows: Record<number, Record<string, string>> = {};
  if (!error || typeof error !== "object") return rows;
  for (const [key, rowError] of Object.entries(error)) {
    const index = Number(key);
    if (!Number.isInteger(index) || index < 0) continue;
    if (!rowError || typeof rowError !== "object") continue;
    const fields: Record<string, string> = {};
    for (const [field, fieldError] of Object.entries(rowError)) {
      if (field === "ref" || field === "type" || field === "types") continue;
      if (field === "message") continue;
      const message = firstNestedErrorMessage(fieldError);
      if (message) fields[field] = message;
    }
    // Row-level message (e.g. from a superRefine addIssue on the row path)
    // with no per-field breakdown.
    if (
      Object.keys(fields).length === 0 &&
      typeof (rowError as { message?: unknown }).message === "string"
    ) {
      fields.root = (rowError as { message: string }).message;
    }
    if (Object.keys(fields).length > 0) rows[index] = fields;
  }
  return rows;
}

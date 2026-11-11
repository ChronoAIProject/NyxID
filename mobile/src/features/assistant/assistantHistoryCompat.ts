import { z } from "zod";

/** Read-only metadata: future strings are unknown, never successful or authorized. */
export function historyEnum<const T extends readonly [string, ...string[]]>(values: T) {
  return z.string().transform((value): T[number] | "unknown" =>
    values.includes(value) ? value as T[number] : "unknown",
  );
}

/** Keep decision schemas strict, but isolate unsupported cards from the transcript. */
export function isolateHistoryItems(
  value: unknown,
  schemas: Record<string, z.ZodType>,
): unknown {
  if (!value || typeof value !== "object" || Array.isArray(value)) return value;
  const result = { ...value } as Record<string, unknown>;
  let unsupported = 0;
  for (const [field, schema] of Object.entries(schemas)) {
    if (!Array.isArray(result[field])) continue;
    result[field] = result[field].filter((item) => {
      const valid = schema.safeParse(item).success;
      if (!valid) unsupported++;
      return valid;
    });
  }
  // This is client-derived only; never trust a server-supplied count.
  delete result.unsupported_history_items;
  if (unsupported) result.unsupported_history_items = unsupported;
  return result;
}

export const HISTORY_UNAVAILABLE = "Conversation history could not be loaded. Please try again.";
export const HISTORY_UNSUPPORTED = "Some activity or confirmation details are unavailable in this app version. Refresh history or update the app.";

// Read-side mapping, audited against ToolFailure producers; see docs/chat/activity-history-compat.md.
export const KNOWN_TOOL_FAILURE_KINDS = [
  "service_status", "tool_error", "refused", "internal",
  "resource_unavailable", "retry_duplicate_action",
] as const;

export function activityPresentationStatus<T extends string>(status: T, kind?: string | null): T | "skipped" | "unknown" {
  if (kind === "retry_duplicate_action") return "skipped";
  if (kind != null && !KNOWN_TOOL_FAILURE_KINDS.some((known) => known === kind) && status !== "error") return "unknown";
  return status;
}

/** Validation diagnostics may contain wire data and are never user-facing copy. */
export function historyErrorMessage(error: Error | null | undefined): string | undefined {
  return error?.name === "ZodError" ? HISTORY_UNAVAILABLE : error?.message;
}

export function parseHistoryResponse<S extends z.ZodType>(schema: S, value: unknown): z.output<S> {
  const result = schema.safeParse(value);
  if (!result.success) throw new Error(HISTORY_UNAVAILABLE);
  return result.data;
}

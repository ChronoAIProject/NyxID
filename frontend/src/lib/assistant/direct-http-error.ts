import type { ApiErrorResponse } from "@/types/api";

export function isNyxidErrorEnvelope(
  value: unknown,
): value is ApiErrorResponse {
  return Boolean(
    value &&
    typeof value === "object" &&
    "error" in value &&
    typeof value.error === "string" &&
    "error_code" in value &&
    typeof value.error_code === "number" &&
    "message" in value &&
    typeof value.message === "string",
  );
}

export async function parseJsonErrorResponse(
  response: Response,
): Promise<unknown> {
  try {
    return await response.json();
  } catch {
    return null;
  }
}

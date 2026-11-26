import { ApiError } from "@/lib/api-error";

/** Refusals that require fresh authority/facts rather than the same consent. */
export function isDefinitiveDraftError(error: unknown, operation: "read" | "adopt") {
  return error instanceof ApiError &&
    (error.status === 403 || error.status === 404 ||
      (operation === "adopt" && error.status === 409));
}

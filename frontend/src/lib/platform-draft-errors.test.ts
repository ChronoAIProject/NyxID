import { expect, it } from "vitest";
import { ApiError } from "@/lib/api-error";
import { isDefinitiveDraftError } from "./platform-draft-errors";

it.each([403, 404, 409, 408, 429, 500, 503, 0])(
  "classifies HTTP %s for reads and adoption",
  (status) => {
    const error = new ApiError(status, {
      error: "refused",
      error_code: -1,
      message: "Refused",
    });
    expect(isDefinitiveDraftError(error, "read")).toBe(
      status === 403 || status === 404,
    );
    expect(isDefinitiveDraftError(error, "adopt")).toBe(
      [403, 404, 409].includes(status),
    );
  },
);

it.each([
  new TypeError("Network unavailable"),
  new Error("Refresh failed"),
  { status: 403 },
  null,
])("keeps non-API failures retryable: %s", (error) => {
  expect(isDefinitiveDraftError(error, "read")).toBe(false);
  expect(isDefinitiveDraftError(error, "adopt")).toBe(false);
});

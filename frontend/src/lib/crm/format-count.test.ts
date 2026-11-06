import { expect, it } from "vitest";
import { formatCrmCount } from "./format-count";

it.each([
  [0, "record", "0 records"],
  [1, "record", "1 record"],
  [2, "record", "2 records"],
  [0, "row", "0 rows"],
  [1, "row", "1 row"],
  [2, "row", "2 rows"],
] as const)("formats %i %s as %s", (count, singular, expected) => {
  expect(formatCrmCount(count, singular)).toBe(expected);
});

import { expect, it } from "vitest";
import {
  toolMetadataFields,
  publicationSchema,
} from "./tools";
it("bounds topics and supplier, rejects duplicates", () => {
  expect(
    toolMetadataFields.topics.safeParse(["web-search", "web-search"]).success,
  ).toBe(false);
  expect(
    toolMetadataFields.topics.safeParse(
      Array.from(
        { length: 21 },
        (_, i) => `topic-${String.fromCharCode(97 + i)}`,
      ),
    ).success,
  ).toBe(false);
  expect(toolMetadataFields.supplier.safeParse("x".repeat(129)).success).toBe(
    false,
  );
  expect(publicationSchema.parse("draft")).toBe("draft");
});

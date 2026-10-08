import { expect, it } from "vitest";
import {
  overlayDocumentSchema,
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
it("validates OpenAPI objects and enforces the upload limit", () => {
  expect(
    overlayDocumentSchema.safeParse({ openapi: "3.1.0", paths: {} }).success,
  ).toBe(true);
  expect(
    overlayDocumentSchema.safeParse({ openapi: "2.0", paths: {} }).success,
  ).toBe(false);
  expect(
    overlayDocumentSchema.safeParse({ openapi: "3.1.0", paths: [] }).success,
  ).toBe(false);
  expect(
    overlayDocumentSchema.safeParse({
      openapi: "3.1.0",
      paths: {},
      description: "x".repeat(1024 * 1024),
    }).success,
  ).toBe(false);
});

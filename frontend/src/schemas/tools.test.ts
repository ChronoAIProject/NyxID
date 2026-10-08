import { expect, it } from "vitest";
import {
  toolMetadataFields,
  publicationSchema,
  addToolSchema,
  importSourceSchema,
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

it("validates twin and new-service paths and catalog provenance", () => {
  const values = {
    name: "Tool",
    slug: "tools-test",
    auth_method: "none",
    auth_key_name: "Authorization",
    twin_of_service_id: "source",
  };
  expect(
    addToolSchema.safeParse({ ...values, creation_mode: "twin" }).success,
  ).toBe(true);
  expect(
    addToolSchema.safeParse({
      ...values,
      creation_mode: "twin",
      twin_of_service_id: "",
    }).success,
  ).toBe(false);
  expect(
    addToolSchema.safeParse({ ...values, creation_mode: "new", base_url: "" })
      .success,
  ).toBe(false);
  expect(
    addToolSchema.safeParse({
      ...values,
      creation_mode: "new",
      base_url: "https://example.com",
    }).success,
  ).toBe(true);
  expect(
    importSourceSchema.parse({ kind: "catalog_twin", reference: "api-twitter" })
      .kind,
  ).toBe("catalog_twin");
});

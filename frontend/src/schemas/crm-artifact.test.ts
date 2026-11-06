import { expect, it } from "vitest";
import { turnCrmArtifactSchema } from "./assistant-nyxagent";

it("requires object keys for record and dataset references and keeps literal wire fields", () => {
  const fields = {
    id: "artifact-id",
    owner_id: "person",
    title: "Asset",
    role: "created",
  };
  for (const kind of ["record", "object"]) {
    expect(turnCrmArtifactSchema.safeParse({ ...fields, kind }).success).toBe(
      false,
    );
    expect(
      turnCrmArtifactSchema.safeParse({ ...fields, kind, object_key: "" })
        .success,
    ).toBe(false);
    const value = { ...fields, kind, object_key: "asset" };
    expect(turnCrmArtifactSchema.parse(value)).toEqual(value);
  }
  expect(turnCrmArtifactSchema.parse({ ...fields, kind: "view" })).toEqual({
    ...fields,
    kind: "view",
  });
});

it("uses the backend's 200 Unicode character title bound", () => {
  const artifact = {
    kind: "view",
    id: "unicode-view",
    owner_id: "person",
    title: "🧪".repeat(200),
    role: "presented",
  };
  expect(turnCrmArtifactSchema.parse(artifact).title).toBe("🧪".repeat(200));
  expect(
    turnCrmArtifactSchema.safeParse({ ...artifact, title: "🧪".repeat(201) })
      .success,
  ).toBe(false);
  expect(
    turnCrmArtifactSchema.safeParse({ ...artifact, title: "x".repeat(201) })
      .success,
  ).toBe(false);
});

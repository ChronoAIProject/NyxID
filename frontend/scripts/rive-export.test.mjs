import { test } from "node:test";
import { strict as assert } from "node:assert";
import { validateRiveExport } from "./rive-export.mjs";

const result = () => ({
  success: true,
  data: { riv: "./build/nyx-emotes.riv" },
  warnings: [],
  workspace: {
    id: "authoring",
    name: "Avatar authoring",
    personal: false,
    admin: true,
  },
});
test("accepts a warning-free export from the expected workspace and records public provenance", () => {
  assert.deepEqual(validateRiveExport(result(), "authoring"), {
    id: "authoring",
    name: "Avatar authoring",
    personal: false,
  });
});
test("rejects warnings even when publishing succeeds", () => {
  for (const warnings of [
    ["watermark"],
    ["other warning"],
    "unexpected warning shape",
  ]) {
    assert.throws(
      () => validateRiveExport({ ...result(), warnings }),
      /warnings/,
    );
  }
});
test("rejects missing or malformed workspace provenance", () => {
  for (const workspace of [
    undefined,
    {},
    { ...result().workspace, personal: "false" },
  ]) {
    assert.throws(
      () => validateRiveExport({ ...result(), workspace }),
      /workspace/,
    );
  }
});
test("rejects the wrong workspace without selecting a workspace", () => {
  assert.throws(
    () => validateRiveExport(result(), "different"),
    /different workspace/,
  );
});
test("rejects unsuccessful or incomplete exports", () => {
  for (const invalid of [
    { ...result(), success: false },
    { ...result(), data: {} },
  ]) {
    assert.throws(() => validateRiveExport(invalid), /export failed/);
  }
});

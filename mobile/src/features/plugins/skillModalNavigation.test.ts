import assert from "node:assert/strict";
import test from "node:test";
import { skillModalBackAction } from "./skillModalNavigation";

test("versions, history and draft return to the skill before closing its modal", () => {
  for (const view of ["versions", "history", "draft"]) {
    assert.equal(skillModalBackAction(view !== "detail", false), "parent");
  }
  assert.equal(skillModalBackAction(false, false), "close");
});

test("authored draft editing returns to its confirmation before returning to chat", () => {
  assert.equal(skillModalBackAction(true, false), "parent");
  assert.equal(skillModalBackAction(false, false), "close");
});

test("a pending mutation blocks both child Back and root dismissal until it settles", () => {
  for (const child of [true, false]) {
    assert.equal(skillModalBackAction(child, true), "stay");
    assert.equal(
      skillModalBackAction(child, false),
      child ? "parent" : "close",
    );
  }
});

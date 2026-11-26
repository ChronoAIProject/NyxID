import assert from "node:assert/strict";
import test from "node:test";
import { chatComposerFocus } from "./chatComposerFocus";

function composer() {
  let blurs = 0;
  return { input: { blur: () => blurs++ }, blurs: () => blurs };
}

test("a touch that starts outside the focused composer blurs it when the touch ends", () => {
  const a = composer();
  chatComposerFocus.focused(a.input);
  chatComposerFocus.workspaceTouchStart();
  assert.equal(a.blurs(), 0, "nothing moves while the finger is down");
  chatComposerFocus.workspaceTouchEnd();
  assert.equal(a.blurs(), 1);
  chatComposerFocus.blurred(a.input);
});

test("touches inside the composer (input, toolbar, mention list) never blur it", () => {
  const a = composer();
  chatComposerFocus.focused(a.input);
  // The composer's handler runs first: touch events bubble inner to outer.
  chatComposerFocus.touchStartInComposer();
  chatComposerFocus.workspaceTouchStart();
  chatComposerFocus.workspaceTouchEnd();
  assert.equal(a.blurs(), 0);
  // The marker belongs to that touch only: the next outside tap blurs.
  chatComposerFocus.workspaceTouchStart();
  chatComposerFocus.workspaceTouchEnd();
  assert.equal(a.blurs(), 1);
  chatComposerFocus.blurred(a.input);
});

test("only the focused composer is blurred, and an unfocused one is left alone", () => {
  const a = composer();
  const b = composer();
  chatComposerFocus.focused(a.input);
  chatComposerFocus.focused(b.input);
  // a's late blur event must not unregister b.
  chatComposerFocus.blurred(a.input);
  chatComposerFocus.workspaceTouchStart();
  chatComposerFocus.workspaceTouchEnd();
  assert.deepEqual([a.blurs(), b.blurs()], [0, 1]);
  chatComposerFocus.blurred(b.input);
  chatComposerFocus.workspaceTouchStart();
  chatComposerFocus.workspaceTouchEnd();
  assert.deepEqual([a.blurs(), b.blurs()], [0, 1], "no composer focused");
  // A touch end without its start (e.g. after a cancel) does nothing.
  chatComposerFocus.focused(a.input);
  chatComposerFocus.workspaceTouchEnd();
  assert.equal(a.blurs(), 0);
  chatComposerFocus.blurred(a.input);
});

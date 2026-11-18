import assert from "node:assert/strict";
import test from "node:test";
import { createWorkspaceFocusState } from "./workspaceFocusState";

function setup() {
  const focused: number[] = [];
  const scheduled: (() => void)[] = [];
  const state = createWorkspaceFocusState(
    (target) => focused.push(target),
    (callback) => scheduled.push(callback),
  );
  return {
    state,
    focused,
    flush: () => {
      while (scheduled.length) scheduled.shift()!();
    },
  };
}
test("dismissal resolves the invoking mounted control after the primary becomes active", () => {
  const { state, focused, flush } = setup();
  const menu = Symbol(),
    back = Symbol();
  state.context("chat", false);
  let tag: number | null = 10;
  state.register(menu, () => tag, true, "chat");
  state.invoked(menu);
  state.register(back, () => 20, true, "chat");
  const origin = state.capture("chat");
  state.context("chat", true);
  state.register(menu, () => tag, false, "chat");
  state.restore(origin);
  flush();
  assert.deepEqual(focused, []);
  state.context("chat", false);
  state.register(menu, () => tag, true, "chat");
  tag = 30;
  flush();
  assert.deepEqual(
    focused,
    [30],
    "read the current native handle rather than a stale layout tag",
  );
  state.register(menu, () => tag, true, "chat");
  flush();
  assert.deepEqual(focused, [30]);
});
test("a recreated primary header receives focus but auth and context replacements cancel restoration", () => {
  for (const nextScope of ["chat", "other-chat", undefined]) {
    const { state, focused, flush } = setup();
    const old = Symbol(),
      replacement = Symbol();
    state.context("chat", false);
    state.register(old, () => 10, true, "chat");
    const origin = state.capture("chat");
    state.context("chat", true);
    state.release(old);
    state.restore(origin);
    state.context(nextScope, false);
    state.register(replacement, () => 20, true, nextScope);
    flush();
    assert.deepEqual(focused, nextScope === "chat" ? [20] : []);
  }
});
test("a released view cannot receive a queued event and restoration never selects an inactive control", () => {
  const { state, focused, flush } = setup();
  const owner = Symbol(),
    inactive = Symbol();
  state.context("chat", false);
  state.register(owner, () => 10, true, "chat");
  const origin = state.capture("chat");
  state.restore(origin);
  state.release(owner);
  state.register(inactive, () => 20, false, "chat");
  flush();
  assert.deepEqual(focused, []);
  state.context("new-chat", false);
  state.register(Symbol(), () => 30, true, "new-chat");
  flush();
  assert.deepEqual(focused, []);
});
test("Close receives focus once per drawer and never after its view or drawer unmounts", () => {
  const { state, focused, flush } = setup();
  let tag: number | null = 10;
  state.focusDrawerOnce("sheet", () => tag);
  state.focusDrawerOnce("sheet", () => 20);
  flush();
  assert.deepEqual(focused, [20]);
  state.focusDrawerOnce("sheet", () => 20);
  flush();
  assert.deepEqual(focused, [20]);
  state.releaseDrawer("sheet");
  state.focusDrawerOnce("sheet", () => tag);
  tag = null;
  flush();
  assert.deepEqual(focused, [20]);
  state.focusDrawerOnce("cancelled", () => 40);
  state.releaseDrawer("cancelled");
  flush();
  assert.deepEqual(focused, [20]);
});

test("auth admission cancels a queued native event and rejects the old drawer origin even if route keys remain", () => {
  let epoch = 1;
  const focused: number[] = [];
  const scheduled: (() => void)[] = [];
  const state = createWorkspaceFocusState(
    (tag) => focused.push(tag),
    (callback) => scheduled.push(callback),
    () => epoch,
  );
  state.context("chat", false);
  state.register(Symbol(), () => 10, true, "chat");
  const origin = state.capture("chat");
  state.restore(origin);
  epoch++;
  while (scheduled.length) scheduled.shift()!();
  state.register(Symbol(), () => 20, true, "chat");
  state.restore(origin);
  while (scheduled.length) scheduled.shift()!();
  assert.deepEqual(focused, []);
});

test("opening navigation cancels a delayed sheet focus return through native modal dismissal", () => {
  const { state, focused, flush } = setup();
  const primary = Symbol("chat header"),
    modal = Symbol("side menu");
  state.context("chat", false);
  state.register(primary, () => 10, true, "chat");
  const origin = state.capture("chat");
  state.context("chat", true);
  state.restore(origin);
  state.context("chat", false); // A callback is now queued after sheet Close.
  state.focusDrawerOnce("queued-sheet", () => 30);
  state.setModalActive(modal, true);
  state.register(primary, () => 20, true, "chat");
  state.restore(origin); // A later unmount cannot revive the sheet request.
  state.focusDrawerOnce("stale-sheet", () => 30);
  flush();
  assert.deepEqual(
    focused,
    [],
    "no underlying header/sheet focus while the menu owns the native modal",
  );
  state.setModalActive(modal, false);
  state.releaseDrawer("queued-sheet");
  flush();
  assert.deepEqual(
    focused,
    [],
    "dismissing the menu cannot revive a cancelled focus request",
  );
  const nextOrigin = state.capture("chat");
  state.context("chat", true);
  state.restore(nextOrigin);
  state.context("chat", false);
  flush();
  assert.deepEqual(
    focused,
    [20],
    "later ordinary sheet dismissal still restores its mounted primary control",
  );
});

test("modal focus ownership survives auth changes and one owner cannot release another", () => {
  let epoch = 1;
  const focused: number[] = [],
    scheduled: (() => void)[] = [];
  const state = createWorkspaceFocusState(
    (target) => focused.push(target),
    (callback) => scheduled.push(callback),
    () => epoch,
  );
  const first = Symbol("menu"),
    second = Symbol("replacement modal");
  state.context("chat", false);
  state.setModalActive(first, true);
  epoch++;
  state.context("chat", false);
  state.setModalActive(second, true);
  state.setModalActive(first, false);
  state.register(Symbol(), () => 40, true, "chat");
  state.restore(state.capture("chat"));
  while (scheduled.length) scheduled.shift()!();
  assert.deepEqual(focused, []);
  state.setModalActive(second, false);
  state.restore(state.capture("chat"));
  while (scheduled.length) scheduled.shift()!();
  assert.deepEqual(focused, [40]);
});

test("a complete menu presentation cancels queued focus even before the scheduler runs", () => {
  const { state, focused, flush } = setup();
  const modal = Symbol("side menu");
  state.context("chat", false);
  state.register(Symbol("chat header"), () => 10, true, "chat");
  state.restore(state.capture("chat"));
  state.focusDrawerOnce("old-sheet", () => 20);
  state.setModalActive(modal, true);
  state.setModalActive(modal, false);
  flush();
  assert.deepEqual(
    focused,
    [],
    "neither a primary nor a Close request can outlive the intervening menu",
  );
  state.releaseDrawer("old-sheet");
  state.focusDrawerOnce("new-sheet", () => 30);
  flush();
  assert.deepEqual(focused, [30]);
});

test("native modal focus requires current ownership and resolves a live target only once", () => {
  const { state, focused, flush } = setup();
  const owner = Symbol("menu");
  state.focusModalOnce(owner, () => 10);
  flush();
  assert.deepEqual(focused, [], "an unpresented menu cannot request focus");
  state.setModalActive(owner, true);
  let target: number | null = null;
  state.focusModalOnce(owner, () => target);
  flush();
  assert.deepEqual(
    focused,
    [],
    "a missing/unmounted control cannot receive focus",
  );
  state.focusModalOnce(owner, () => target);
  target = 20;
  flush();
  assert.deepEqual(focused, [20]);
  state.focusModalOnce(owner, () => 30);
  flush();
  assert.deepEqual(
    focused,
    [20],
    "focus is requested once per native presentation",
  );
  state.setModalActive(owner, false);
  state.setModalActive(owner, true);
  state.focusModalOnce(owner, () => 40);
  state.setModalActive(owner, false);
  state.setModalActive(owner, true);
  flush();
  assert.deepEqual(
    focused,
    [20],
    "a new presentation rejects the old queued request",
  );
  state.focusModalOnce(owner, () => 50);
  flush();
  assert.deepEqual(focused, [20, 50]);
});

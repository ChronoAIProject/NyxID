import assert from "node:assert/strict";
import test from "node:test";
import { createMfaRecoveryLifecycle } from "./mfaRecoveryLifecycle";

function harness() {
  let authEpoch = 1;
  const lifecycle = createMfaRecoveryLifecycle(() => authEpoch);
  lifecycle.setFocused(true);
  lifecycle.setActive(true);
  return {
    lifecycle,
    changeAccount() {
      authEpoch += 1;
    },
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test("cancelled Android sharing and genuine background mask rather than lose unsaved codes", () => {
  const { lifecycle } = harness();
  const confirmation = lifecycle.beginConfirmation()!;
  lifecycle.completeConfirmation(confirmation, ["first", "second"]);

  // Android's chooser can background the Activity. Its result is not proof
  // that the user saved anything, and a subsequent Home can do the same.
  for (let i = 0; i < 2; i += 1) {
    lifecycle.setActive(false);
    assert.deepEqual(lifecycle.snapshot(), {
      codes: [],
      count: 2,
      confirming: false,
    });
    lifecycle.setActive(true);
    assert.deepEqual(lifecycle.snapshot().codes, ["first", "second"]);
  }
  lifecycle.clear();
  assert.equal(lifecycle.snapshot().count, 0);
});

test("a held confirm stays protected across background and reveals its output only on foreground", async () => {
  const { lifecycle } = harness();
  const response = deferred<readonly string[]>();
  const confirmation = lifecycle.beginConfirmation()!;
  const settled = response.promise.then((codes) =>
    lifecycle.completeConfirmation(confirmation, codes),
  );
  assert.equal(lifecycle.snapshot().confirming, true);
  assert.equal(
    lifecycle.beginConfirmation(),
    null,
    "double submit must not create a second confirmation",
  );
  lifecycle.setActive(false);
  assert.equal(
    lifecycle.snapshot().confirming,
    true,
    "backgrounding must not release the navigation guard",
  );
  response.resolve(["only-once"]);
  assert.equal(await settled, true);
  assert.deepEqual(
    lifecycle.snapshot().codes,
    [],
    "late background output is never exposed",
  );
  lifecycle.setActive(true);
  assert.deepEqual(lifecycle.snapshot().codes, ["only-once"]);
  assert.equal(lifecycle.snapshot().confirming, false);
});

test("leaving and returning cannot accept a late enrollment response or retain prior output", async () => {
  const { lifecycle } = harness();
  const response = deferred<readonly string[]>();
  const confirmation = lifecycle.beginConfirmation()!;
  const settled = response.promise.then((codes) =>
    lifecycle.completeConfirmation(confirmation, codes),
  );
  lifecycle.setFocused(false);
  lifecycle.setFocused(true);
  response.resolve(["old-screen"]);
  assert.equal(await settled, false);
  assert.deepEqual(lifecycle.snapshot(), {
    codes: [],
    count: 0,
    confirming: false,
  });
  const next = lifecycle.beginConfirmation()!;
  lifecycle.completeConfirmation(next, ["new-screen"]);
  lifecycle.setFocused(false);
  lifecycle.setFocused(true);
  assert.equal(lifecycle.snapshot().count, 0);
});

test("account changes reject both late confirm and previously received recovery material", () => {
  const first = harness();
  const pending = first.lifecycle.beginConfirmation()!;
  first.changeAccount();
  assert.equal(
    first.lifecycle.completeConfirmation(pending, ["wrong-account"]),
    false,
  );
  assert.equal(first.lifecycle.beginConfirmation(), null);
  assert.deepEqual(first.lifecycle.snapshot(), {
    codes: [],
    count: 0,
    confirming: false,
  });

  const second = harness();
  const done = second.lifecycle.beginConfirmation()!;
  second.lifecycle.completeConfirmation(done, ["previous-account"]);
  second.changeAccount();
  assert.equal(second.lifecycle.snapshot().count, 0);
  assert.deepEqual(second.lifecycle.snapshot().codes, []);
});

test("failed confirmation releases protection and permits a new attempt without reviving old tickets", () => {
  const { lifecycle } = harness();
  const failed = lifecycle.beginConfirmation()!;
  assert.equal(lifecycle.finishConfirmation(failed), true);
  assert.equal(lifecycle.snapshot().confirming, false);
  const retry = lifecycle.beginConfirmation()!;
  assert.equal(lifecycle.completeConfirmation(failed, ["stale"]), false);
  assert.equal(lifecycle.snapshot().confirming, true);
  lifecycle.completeConfirmation(retry, ["retry"]);
  assert.deepEqual(lifecycle.snapshot().codes, ["retry"]);
});

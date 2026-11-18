import assert from "node:assert/strict";
import { getEventListeners } from "node:events";
import test from "node:test";
import {
  VOICE_RECOVERY_TIMEOUT_MS,
  withVoiceRequestDeadline,
} from "./voiceRequestDeadline";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

test("a hung request starts synchronously and times out at the default deadline even if it ignores abort", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  assert.equal(VOICE_RECOVERY_TIMEOUT_MS, 8_000);
  const controller = new AbortController();
  let started = false;
  let settled = false;
  const result = withVoiceRequestDeadline(controller, () => {
    started = true;
    return new Promise<never>(() => {});
  });
  assert.equal(started, true);
  void result.then(
    () => (settled = true),
    () => (settled = true),
  );
  let timeout: unknown;
  const rejected = assert.rejects(result, (error) => {
    timeout = error;
    return (
      error instanceof Error &&
      error.name === "VoiceRequestTimeoutError" &&
      error.message === "Voice recovery request timed out"
    );
  });
  context.mock.timers.tick(7_999);
  await flush();
  assert.equal(settled, false);
  assert.equal(controller.signal.aborted, false);
  context.mock.timers.tick(1);
  await rejected;
  assert.equal(controller.signal.aborted, true);
  assert.equal(controller.signal.reason, timeout);
  assert.deepEqual(getEventListeners(controller.signal, "abort"), []);
});

test("a caller abort immediately rejects a hung request and clears its deadline and listener", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const controller = new AbortController();
  const abort = context.mock.method(controller, "abort");
  const result = withVoiceRequestDeadline(
    controller,
    () => new Promise<never>(() => {}),
  );
  const rejected = assert.rejects(result, { name: "AbortError" });
  controller.abort("caller cancelled");
  await rejected;
  assert.equal(controller.signal.reason, "caller cancelled");
  assert.deepEqual(getEventListeners(controller.signal, "abort"), []);
  context.mock.timers.tick(VOICE_RECOVERY_TIMEOUT_MS);
  await flush();
  assert.equal(abort.mock.callCount(), 1);
});

test("an already aborted request performs no operation or timer setup", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const timer = context.mock.method(globalThis, "setTimeout");
  const controller = new AbortController();
  controller.abort();
  let calls = 0;
  await assert.rejects(
    withVoiceRequestDeadline(controller, async () => ++calls),
    { name: "AbortError" },
  );
  assert.equal(calls, 0);
  assert.equal(timer.mock.callCount(), 0);
  assert.deepEqual(getEventListeners(controller.signal, "abort"), []);
});

test("successful requests preserve their value and cannot be aborted by a leftover deadline", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const controller = new AbortController();
  const request = deferred<{ session: string }>();
  const result = withVoiceRequestDeadline(
    controller,
    () => request.promise,
    25,
  );
  context.mock.timers.tick(24);
  const value = { session: "confirmed-session" };
  request.resolve(value);
  assert.equal(await result, value);
  assert.deepEqual(getEventListeners(controller.signal, "abort"), []);
  context.mock.timers.tick(1_000);
  await flush();
  assert.equal(controller.signal.aborted, false);
});

test("operation failures preserve the original error and release deadline resources", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  for (const synchronous of [false, true]) {
    const controller = new AbortController();
    const failure = new Error("status unavailable");
    const result = withVoiceRequestDeadline(controller, () => {
      if (synchronous) throw failure;
      return Promise.reject(failure);
    });
    await assert.rejects(result, (error) => error === failure);
    assert.deepEqual(getEventListeners(controller.signal, "abort"), []);
    context.mock.timers.tick(VOICE_RECOVERY_TIMEOUT_MS);
    await flush();
    assert.equal(controller.signal.aborted, false);
  }
});

for (const cancellation of ["timeout", "abort"] as const) {
  for (const completion of ["resolve", "reject"] as const) {
    test(`a late ${completion} after ${cancellation} stays handled and cannot change the result`, async (context) => {
      context.mock.timers.enable({ apis: ["setTimeout"] });
      const controller = new AbortController();
      const request = deferred<string>();
      const result = withVoiceRequestDeadline(
        controller,
        () => request.promise,
        10,
      );
      const name =
        cancellation === "timeout" ? "VoiceRequestTimeoutError" : "AbortError";
      const rejected = assert.rejects(result, { name });
      if (cancellation === "timeout") context.mock.timers.tick(10);
      else controller.abort();
      await rejected;
      if (completion === "resolve") request.resolve("late status");
      else request.reject(new Error("late transport failure"));
      await flush();
      await assert.rejects(result, { name });
      assert.deepEqual(getEventListeners(controller.signal, "abort"), []);
    });
  }
}

test("abort during synchronous operation startup wins even if the operation returns an already resolved promise", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const controller = new AbortController();
  const result = withVoiceRequestDeadline(controller, () => {
    controller.abort();
    return Promise.resolve("ignored status");
  });
  await assert.rejects(result, { name: "AbortError" });
  context.mock.timers.tick(VOICE_RECOVERY_TIMEOUT_MS);
  await flush();
  assert.deepEqual(getEventListeners(controller.signal, "abort"), []);
});

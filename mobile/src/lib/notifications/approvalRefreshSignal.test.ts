import assert from "node:assert/strict";
import test from "node:test";

import { createApprovalRefreshSignal } from "./approvalRefreshSignal";

class FakeTimers {
  nowMs = 0;
  private nextId = 1;
  private readonly timers = new Map<
    number,
    { callback: () => void; runAt: number }
  >();

  readonly setTimer = (callback: () => void, delayMs: number): number => {
    const id = this.nextId++;
    this.timers.set(id, { callback, runAt: this.nowMs + delayMs });
    return id;
  };

  readonly clearTimer = (timer: unknown): void => {
    this.timers.delete(timer as number);
  };

  get pendingTimerCount(): number {
    return this.timers.size;
  }

  advanceBy(durationMs: number): void {
    const target = this.nowMs + durationMs;

    while (true) {
      const next = [...this.timers.entries()]
        .filter(([, timer]) => timer.runAt <= target)
        .sort(([, a], [, b]) => a.runAt - b.runAt)[0];
      if (!next) break;

      const [id, timer] = next;
      this.timers.delete(id);
      this.nowMs = timer.runAt;
      timer.callback();
    }

    this.nowMs = target;
  }
}

function createHarness() {
  const timers = new FakeTimers();
  const signal = createApprovalRefreshSignal({
    throttleMs: 1_000,
    now: () => timers.nowMs,
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });
  return { signal, timers };
}

test("delivers the first signal on the leading edge", () => {
  const { signal, timers } = createHarness();
  const deliveries: number[] = [];
  signal.subscribe(() => deliveries.push(timers.nowMs));

  signal.signal();

  assert.deepEqual(deliveries, [0]);
  assert.equal(timers.pendingTimerCount, 0);
});

test("coalesces a burst into at most one delivery per throttle window", () => {
  const { signal, timers } = createHarness();
  const deliveries: number[] = [];
  signal.subscribe(() => deliveries.push(timers.nowMs));

  signal.signal();
  timers.advanceBy(100);
  signal.signal();
  timers.advanceBy(200);
  signal.signal();
  timers.advanceBy(699);

  assert.deepEqual(deliveries, [0]);
  assert.equal(timers.pendingTimerCount, 1);

  timers.advanceBy(1);
  assert.deepEqual(deliveries, [0, 1_000]);
  assert.equal(timers.pendingTimerCount, 0);
});

test("delivers a trailing catch-up for the final in-window signal", () => {
  const { signal, timers } = createHarness();
  const deliveries: number[] = [];
  signal.subscribe(() => deliveries.push(timers.nowMs));

  signal.signal();
  timers.advanceBy(999);
  signal.signal();
  timers.advanceBy(1);

  assert.deepEqual(deliveries, [0, 1_000]);

  timers.advanceBy(1_000);
  assert.deepEqual(deliveries, [0, 1_000]);
});

test("unsubscribing the last listener clears its timer without dropping catch-up", () => {
  const { signal, timers } = createHarness();
  const deliveries: number[] = [];
  const unsubscribe = signal.subscribe(() => deliveries.push(timers.nowMs));

  signal.signal();
  timers.advanceBy(100);
  signal.signal();
  assert.equal(timers.pendingTimerCount, 1);

  unsubscribe();
  assert.equal(timers.pendingTimerCount, 0);
  timers.advanceBy(1_900);
  assert.deepEqual(deliveries, [0]);

  signal.subscribe(() => deliveries.push(timers.nowMs));
  assert.deepEqual(deliveries, [0, 2_000]);
});

test("clear cancels a trailing timer and discards pending work", () => {
  const { signal, timers } = createHarness();
  const deliveries: number[] = [];
  signal.subscribe(() => deliveries.push(timers.nowMs));

  signal.signal();
  timers.advanceBy(100);
  signal.signal();
  assert.equal(timers.pendingTimerCount, 1);

  signal.clear();
  assert.equal(timers.pendingTimerCount, 0);
  timers.advanceBy(1_000);
  assert.deepEqual(deliveries, [0]);
});

test("clear starts a fresh throttle window for the next auth session", () => {
  const { signal, timers } = createHarness();
  const deliveries: number[] = [];
  signal.subscribe(() => deliveries.push(timers.nowMs));

  signal.signal();
  timers.advanceBy(100);
  signal.clear();
  signal.signal();

  assert.deepEqual(deliveries, [0, 100]);
  assert.equal(timers.pendingTimerCount, 0);
});

test("one throwing listener does not block the remaining listeners", () => {
  const { signal } = createHarness();
  let laterListenerCalls = 0;

  signal.subscribe(() => {
    throw new Error("listener failed");
  });
  signal.subscribe(() => {
    laterListenerCalls += 1;
  });

  assert.doesNotThrow(() => signal.signal());
  assert.equal(laterListenerCalls, 1);
});

test("supports repeated subscribe and unsubscribe cycles without duplicate delivery", () => {
  const { signal, timers } = createHarness();
  let firstListenerCalls = 0;
  let secondListenerCalls = 0;

  const stopFirst = signal.subscribe(() => {
    firstListenerCalls += 1;
  });
  signal.signal();
  stopFirst();

  timers.advanceBy(1_000);
  signal.signal();
  const stopSecond = signal.subscribe(() => {
    secondListenerCalls += 1;
  });
  stopSecond();

  timers.advanceBy(1_000);
  const stopThird = signal.subscribe(() => {
    secondListenerCalls += 1;
  });
  signal.signal();
  stopThird();

  assert.equal(firstListenerCalls, 1);
  assert.equal(secondListenerCalls, 2);
  assert.equal(timers.pendingTimerCount, 0);
});

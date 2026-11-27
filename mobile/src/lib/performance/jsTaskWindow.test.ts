import assert from "node:assert/strict";
import test from "node:test";
import { createJsTaskObserver, type JsTaskTiming } from "./jsTaskWindow";

type Entry = { startTime: number; duration: number };
type ObserverType = NonNullable<
  ReturnType<NonNullable<Parameters<typeof createJsTaskObserver>[2]>>
>;
function fixture(type?: ObserverType) {
  let epoch: number | null = null;
  let active = true;
  let sampled = true;
  let begins = 0;
  const reports: JsTaskTiming[] = [];
  const monitor = createJsTaskObserver(
    {
      active: () => active,
      current: () => epoch,
      begin: () => {
        begins++;
        return sampled ? epoch : null;
      },
      submit: (_epoch, timing) => reports.push(timing),
    },
    Date.now,
    () => type,
  );
  return {
    monitor,
    reports,
    begins: () => begins,
    setEpoch(value: number | null) {
      epoch = value;
      monitor.update();
    },
    setActive(value: boolean) {
      active = value;
      monitor.update();
    },
    sample(value: boolean) {
      sampled = value;
    },
  };
}

test("RN native entries honor the grant boundary, drain the final queue, and report once per active window", (t) => {
  t.mock.timers.enable({ apis: ["Date", "setTimeout"], now: 1000 });
  let publish!: (entries: Entry[]) => void;
  let closed = 0;
  class Native {
    static supportedEntryTypes = ["longtask"];
    constructor(fn: (list: { getEntries(): Entry[] }) => void) {
      publish = (entries) => fn({ getEntries: () => entries });
    }
    observe(options: unknown) {
      assert.deepEqual(options, { type: "longtask", buffered: false });
    }
    takeRecords() {
      return [{ startTime: 5000, duration: 125 }];
    }
    disconnect() {
      closed++;
    }
  }
  const x = fixture(Native);
  x.monitor.update();
  assert.equal(x.begins(), 0);
  x.setEpoch(1);
  publish([
    { startTime: 800, duration: 100 },
    { startTime: 1300, duration: 60 },
  ]);
  t.mock.timers.tick(10_000);
  assert.deepEqual(x.reports, [
    {
      observation_source: "rn_longtask",
      duration_ms: 10000,
      threshold_ms: 50,
      task_count: 2,
      blocked_ms: 85,
      max_task_ms: 125,
    },
  ]);
  x.monitor.update();
  publish([{ startTime: 10800, duration: 90 }]);
  t.mock.timers.tick(10_000);
  assert.equal(closed, 1);
  assert.equal(x.begins(), 1);
});

test("the lag fallback reports observed timer delay in its own fields", (t) => {
  t.mock.timers.enable({ apis: ["Date", "setTimeout"], now: 0 });
  class Unsupported {
    static supportedEntryTypes = ["longtask"];
    observe() {
      throw Error("unsupported at runtime");
    }
    takeRecords() {
      return [];
    }
    disconnect() {}
  }
  const x = fixture(Unsupported);
  x.setEpoch(1);
  t.mock.timers.tick(350);
  for (let i = 0; i < 39; i++) t.mock.timers.tick(250);
  assert.deepEqual(x.reports, [
    {
      observation_source: "event_loop_lag",
      duration_ms: 10100,
      threshold_ms: 50,
      sample_count: 40,
      tick_interval_ms: 250,
      stall_count: 1,
      max_lag_ms: 100,
      lag_over_threshold_ms: 50,
    },
  ]);
});

test("repeated updates, background, opt-out, unmount and failed reads cannot turn missing coverage into zero tasks", (t) => {
  t.mock.timers.enable({ apis: ["Date", "setTimeout"], now: 1000 });
  let starts = 0;
  let readable = false;
  class Native {
    static supportedEntryTypes = ["longtask"];
    observe() {
      starts++;
    }
    takeRecords() {
      if (!readable) throw Error("unavailable");
      return [];
    }
    disconnect() {}
  }
  const x = fixture(Native);
  x.sample(false);
  x.setEpoch(1);
  x.monitor.update();
  assert.equal(x.begins(), 1);
  assert.equal(starts, 0);
  x.setActive(false);
  x.sample(true);
  x.setActive(true);
  t.mock.timers.tick(1000);
  x.setEpoch(null);
  t.mock.timers.tick(10000);
  x.setEpoch(2);
  x.setActive(false);
  t.mock.timers.tick(10000);
  x.setActive(true);
  t.mock.timers.tick(10000);
  assert.deepEqual(x.reports, []);
  readable = true;
  x.setActive(false);
  x.setActive(true);
  t.mock.timers.tick(10000);
  assert.deepEqual(x.reports, [
    {
      observation_source: "rn_longtask",
      duration_ms: 10000,
      threshold_ms: 50,
      task_count: 0,
      blocked_ms: 0,
    },
  ]);
  x.setActive(false);
  x.setActive(true);
  x.monitor.cancel();
  t.mock.timers.tick(10000);
  assert.equal(x.reports.length, 1);
});

test("a failed lazy native observer lookup selects the timer source", (t) => {
  t.mock.timers.enable({ apis: ["Date", "setTimeout"], now: 0 });
  const reports: JsTaskTiming[] = [];
  const observer = createJsTaskObserver(
    {
      active: () => true,
      current: () => 1,
      begin: () => 1,
      submit: (_epoch, result) => reports.push(result),
    },
    Date.now,
    () => {
      throw new Error("unavailable");
    },
  );
  observer.update();
  for (let n = 0; n < 40; n++) t.mock.timers.tick(250);
  assert.equal(reports[0]?.observation_source, "event_loop_lag");
  observer.cancel();
});

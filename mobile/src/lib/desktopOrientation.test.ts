import assert from "node:assert/strict";
import test from "node:test";
import type { OrientationLock } from "expo-screen-orientation";
import { createDesktopOrientation } from "./desktopOrientation";
import { deferred } from "../features/assistant/assistantHookTestHarness";

const locks = { LANDSCAPE: 5, PORTRAIT_UP: 3 } as const;
function setup(lockAsync: (lock: number) => Promise<void>) {
  let loads = 0;
  const orientation = createDesktopOrientation(async () => {
    loads++;
    return { lockAsync, OrientationLock: locks as unknown as typeof OrientationLock };
  });
  return { ...orientation, loads: () => loads };
}

test("orientation loads lazily and uses the exact Expo landscape and upright portrait locks", async () => {
  const calls: number[] = [];
  const x = setup(async (lock) => { calls.push(lock); });
  assert.equal(x.loads(), 0);
  await x.lockDesktopLandscape();
  await x.restorePortrait();
  assert.deepEqual(calls, [locks.LANDSCAPE, locks.PORTRAIT_UP]);
});

test("blur/background portrait waits for a late mount landscape lock", async () => {
  const gate = deferred<void>();
  const calls: number[] = [];
  let applied: number | undefined;
  let concurrent = 0;
  const x = setup(async (lock) => {
    assert.equal(concurrent++, 0, "native orientation operations never overlap");
    calls.push(lock);
    if (lock === locks.LANDSCAPE) await gate.promise;
    applied = lock;
    concurrent--;
  });
  const entering = x.lockDesktopLandscape();
  await Promise.resolve();
  await Promise.resolve();
  const leaving = x.restorePortrait();
  assert.deepEqual(calls, [locks.LANDSCAPE]);
  gate.resolve();
  await Promise.all([entering, leaving]);
  assert.equal(applied, locks.PORTRAIT_UP);
  assert.deepEqual(calls, [locks.LANDSCAPE, locks.PORTRAIT_UP]);
});

test("rapid enter/leave/resume preserves request order and the final portrait restore", async () => {
  const calls: number[] = [];
  const x = setup(async (lock) => { calls.push(lock); });
  await Promise.all([
    x.lockDesktopLandscape(), x.restorePortrait(),
    x.lockDesktopLandscape(), x.restorePortrait(), x.restorePortrait(),
  ]);
  assert.deepEqual(calls, [5, 3, 5, 3, 3]);
});

test("real native errors reject their caller while queued cleanup and retry still run", async () => {
  const failure = new Error("Native orientation unavailable");
  const calls: number[] = [];
  const x = setup(async (lock) => {
    calls.push(lock);
    if (calls.length === 1) throw failure;
  });
  const entering = x.lockDesktopLandscape();
  const leaving = x.restorePortrait();
  await assert.rejects(entering, (error) => error === failure);
  await leaving;
  await x.lockDesktopLandscape();
  await x.restorePortrait();
  assert.deepEqual(calls, [5, 3, 5, 3]);
});

test("a failed module load rejects and does not block a later portrait restore", async () => {
  let loads = 0;
  const calls: number[] = [];
  const x = createDesktopOrientation(async () => {
    if (++loads === 1) throw new Error("Missing native module");
    return { lockAsync: async (lock) => { calls.push(lock); }, OrientationLock: locks as unknown as typeof OrientationLock };
  });
  await assert.rejects(x.lockDesktopLandscape(), /Missing native module/);
  await x.restorePortrait();
  assert.deepEqual(calls, [3]);
});

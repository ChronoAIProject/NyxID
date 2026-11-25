import assert from "node:assert/strict";
import test from "node:test";
import { createPerformanceConsent } from "./consent";

test("only a persisted grant for the current destination enables observations after hydration", async () => {
  for (const value of [null, "other", "project"]) {
    const consent = createPerformanceConsent(
      "project",
      async () => value,
      async () => {},
      () => true,
    );
    assert.equal(consent.begin(), null);
    await consent.hydrate();
    assert.equal(consent.current(), value === "project" ? 1 : null);
    await consent.set(true);
    assert.notEqual(consent.begin(), null);
  }
});

test("late hydration cannot resurrect a withdrawn grant and a regrant uses a fresh epoch", async () => {
  let finish!: (value: string) => void;
  const writes: Array<string | null> = [];
  const consent = createPerformanceConsent(
    "project",
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
    async (value) => {
      writes.push(value);
    },
    () => true,
  );
  const hydration = consent.hydrate();
  const denied = consent.set(false);
  finish("project");
  await denied;
  await hydration;
  assert.equal(consent.current(), null);
  await consent.set(true);
  const epoch = consent.begin();
  const off = consent.set(false);
  const on = consent.set(true);
  assert.equal(consent.current(), null);
  await Promise.all([off, on]);
  assert.deepEqual(writes, [null, "project", null, "project"]);
  assert.notEqual(consent.current(), epoch);
});

test("failed reads can retry and failed withdrawal writes immediately stop this session", async () => {
  let reads = 0;
  const consent = createPerformanceConsent(
    "project",
    async () => {
      if (++reads === 1) throw new Error("read");
      return "project";
    },
    async () => {
      throw new Error("write");
    },
    () => false,
  );
  await assert.rejects(consent.hydrate(), /read/);
  await consent.hydrate();
  assert.notEqual(consent.current(), null);
  assert.equal(consent.begin(), null);
  await assert.rejects(consent.set(false), /write/);
  assert.equal(consent.current(), null);
  await assert.rejects(consent.set(true), /write/);
  assert.equal(consent.current(), null);
});

test("a missing project does not access storage or sample", async () => {
  const unexpected = () => {
    throw new Error("unexpected");
  };
  const consent = createPerformanceConsent(
    null,
    unexpected,
    unexpected,
    unexpected,
  );
  await consent.hydrate();
  await consent.set(true);
  assert.equal(consent.begin(), null);
});

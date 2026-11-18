import test from "node:test";
import assert from "node:assert/strict";
import { readBoundedBytes } from "./readBoundedBytes";

test("download preserves binary chunks up to the exact limit", async () => {
  const stream = new ReadableStream<Uint8Array>({
    start(c) {
      c.enqueue(new Uint8Array([0, 255]));
      c.enqueue(new Uint8Array([40]));
      c.close();
    },
  });
  assert.deepEqual(
    await readBoundedBytes(stream, 3),
    new Uint8Array([0, 255, 40]),
  );
  assert.equal(stream.locked, false);
});
test("overflow cancels before consuming the rest of a large download", async () => {
  let cancelled = false;
  const stream = new ReadableStream<Uint8Array>({
    start(c) {
      c.enqueue(new Uint8Array(4));
    },
    cancel() {
      cancelled = true;
    },
  });
  await assert.rejects(readBoundedBytes(stream, 3), /download limit/);
  assert.equal(cancelled, true);
  assert.equal(stream.locked, false);
});
test("failed or absent native body never becomes a cached partial attachment", async () => {
  await assert.rejects(readBoundedBytes(null, 3), /unavailable/);
  const stream = new ReadableStream<Uint8Array>({
    start(c) {
      c.error(new Error("network"));
    },
  });
  await assert.rejects(readBoundedBytes(stream, 3), /network/);
  assert.equal(stream.locked, false);
});

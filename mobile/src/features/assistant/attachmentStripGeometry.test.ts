import assert from "node:assert/strict";
import test from "node:test";
import { attachmentStripGeometry } from "./attachmentStripGeometry";

test("three square tiles fit the measured viewport including padding/gaps and fractional widths", () => {
  for (const width of [220, 260, 300.25, 343.5, 734, 1176.75]) {
    for (let count = 1; count <= 10; count++) {
      const first = attachmentStripGeometry(width, count, 0);
      assert.ok(first.tile * 3 + 24 <= width);
      assert.ok(first.tile * 3 + 24 > width - 3);
      assert.equal(first.overflow, count > 3);
      assert.equal(first.before, false);
      assert.equal(first.after, count > 3);
      assert.equal(first.start, 1);
      assert.equal(first.end, Math.min(count, 3));
      const last = attachmentStripGeometry(width, count, Infinity);
      assert.equal(last.after, false);
      assert.equal(last.end, count);
      if (count > 3) {
        assert.equal(last.start, count - 2);
        // The final tile's trailing edge fits inside the viewport at the end.
        assert.ok(last.content - 4 <= last.offset + width);
      }
    }
  }
});

test("swipes report partially visible tiles, exclude padding/gaps and clamp overscroll", () => {
  const width = 343.5;
  const first = attachmentStripGeometry(width, 10, 0);
  const stride = first.tile + 8;
  assert.deepEqual(
    [
      attachmentStripGeometry(width, 10, stride).start,
      attachmentStripGeometry(width, 10, stride).end,
    ],
    [2, 4],
  );
  assert.equal(attachmentStripGeometry(width, 10, first.tile + 4).start, 2);
  assert.equal(attachmentStripGeometry(width, 10, first.tile + 3).start, 1);
  assert.equal(attachmentStripGeometry(width, 10, -100).offset, 0);
  assert.equal(
    attachmentStripGeometry(width, 10, 100000).offset,
    first.maxOffset,
  );
});

test("resize, removal and additions keep ranges within the live selection", () => {
  const old = attachmentStripGeometry(734, 10, Infinity);
  const removed = attachmentStripGeometry(734, 4, old.offset);
  assert.equal(removed.offset, removed.maxOffset);
  assert.deepEqual([removed.start, removed.end], [2, 4]);
  const fits = attachmentStripGeometry(734, 3, old.offset);
  assert.equal(fits.offset, 0);
  assert.equal(fits.overflow, false);
  assert.equal(fits.before || fits.after, false);
  const added = attachmentStripGeometry(734, 10, removed.offset);
  assert.equal(added.end, 4);
  assert.equal(added.after, true);
  const resized = attachmentStripGeometry(300, 10, old.offset);
  assert.ok(resized.start <= resized.end && resized.end <= 10);
  assert.equal(resized.end, 10);
  assert.equal(attachmentStripGeometry(0, 10, 100).overflow, false);
});

test("compact composers keep tap targets and expose scrolling even with two tiles", () => {
  const first = attachmentStripGeometry(130, 2, 0);
  assert.equal(first.tile, 64);
  assert.equal(first.overflow, true);
  assert.equal(first.after, true);
  const last = attachmentStripGeometry(130, 2, Infinity);
  assert.equal(last.before, true);
  assert.equal(last.after, false);
  assert.equal(last.end, 2);
  assert.ok(last.content - 4 <= last.offset + 130);
});

import assert from "node:assert/strict";
import test from "node:test";
import { transcriptEndOffset } from "./assistantTranscriptScroll";

test("a short persisted reply remains visible after the live footer shrinks", () => {
  const viewport = 420;
  const liveContent = 900;
  assert.equal(transcriptEndOffset(liveContent, viewport), 480);
  const savedContent = 260;
  assert.equal(transcriptEndOffset(savedContent, viewport), 0);
});

test("following variable-height reply growth uses its measured bottom", () => {
  const viewport = 400;
  assert.deepEqual(
    [180, 430, 710, 560].map((height) => transcriptEndOffset(height, viewport)),
    [0, 30, 310, 160],
  );
  assert.equal(transcriptEndOffset(560, 280), 280);
});

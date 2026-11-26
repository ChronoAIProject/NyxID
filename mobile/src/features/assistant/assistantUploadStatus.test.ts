import assert from "node:assert/strict";
import test from "node:test";
import { assistantUploadStatus } from "./assistantUploadStatus";

for (const [label, row, expected] of [
  [
    "waiting",
    { progress: 0 },
    { text: "Waiting to upload…", loader: "indeterminate", progress: 0 },
  ],
  [
    "uploading",
    { progress: 42 },
    { text: "Uploading 42%", loader: "determinate", progress: 42 },
  ],
  [
    "checking",
    { progress: 99 },
    { text: "Checking file…", loader: "indeterminate", progress: 99 },
  ],
  ["ready", { progress: 100, item: {} }, { loader: "none", progress: 100 }],
  [
    "error",
    { progress: 42, error: "Upload failed" },
    { text: "Upload failed", loader: "none", progress: 0 },
  ],
  [
    "nonfinite",
    { progress: NaN },
    { text: "Waiting to upload…", loader: "indeterminate", progress: 0 },
  ],
] as const) {
  test(`upload status maps ${label}`, () =>
    assert.deepEqual(assistantUploadStatus(row as any), expected));
}

test("an invalid receipt is never complete and explicit removal exposes busy feedback", () => {
  const item = { id: "old", label: "old", content_type: "text/plain", size: 1 };
  assert.deepEqual(assistantUploadStatus({ item, needsReupload: true, progress: 0 }), {
    text: "Waiting to upload…", loader: "indeterminate", progress: 0,
  });
  assert.deepEqual(assistantUploadStatus({ item, removing: true, progress: 100 }), {
    text: "Removing attachment…", loader: "indeterminate", progress: 0,
  });
});

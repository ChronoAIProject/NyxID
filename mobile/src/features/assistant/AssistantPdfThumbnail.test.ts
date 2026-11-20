import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";

for (const installed of [true, false]) {
  test(`PDF native view ${installed ? "loads by its registered name" : "falls back in older native binaries"}`, () => {
    const required: string[] = [];
    const harness = hookHarness({
      expo: {
        requireOptionalNativeModule: (name: string) => {
          assert.equal(name, "NyxPdfThumbnail");
          return installed ? {} : null;
        },
        requireNativeView: (name: string) => {
          required.push(name);
          return "PdfView";
        },
      },
    });
    const loaded = harness.load(
      fileURLToPath(new URL("./AssistantPdfThumbnail.tsx", import.meta.url)),
    );
    assert.equal(loaded.AssistantPdfThumbnail, installed ? "PdfView" : null);
    assert.deepEqual(required, installed ? ["NyxPdfThumbnail"] : []);
  });
}

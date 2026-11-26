import assert from "node:assert/strict";
import test from "node:test";
import { cachePublicConfigHints, getCrashDiagnosticsOffered, subscribePublicConfigHints } from "./publicConfigHints";

test("cached public hints are optional, fail closed and notify only on rollout changes", () => {
  cachePublicConfigHints({});
  let changes = 0;
  const unsubscribe = subscribePublicConfigHints(() => { changes++; });
  for (const value of [null, {}, { crash_diagnostics_offered: "true" }]) cachePublicConfigHints(value);
  assert.equal(getCrashDiagnosticsOffered(), false);
  assert.equal(changes, 0);
  cachePublicConfigHints({ crash_diagnostics_offered: true });
  cachePublicConfigHints({ crash_diagnostics_offered: true });
  assert.equal(getCrashDiagnosticsOffered(), true);
  assert.equal(changes, 1);
  cachePublicConfigHints({});
  assert.equal(getCrashDiagnosticsOffered(), false);
  assert.equal(changes, 2);
  unsubscribe();
});

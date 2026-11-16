import assert from "node:assert/strict";
import test from "node:test";
import { auditCatalogs, discoverPortions } from "./lib/i18n-catalogs.mjs";

test("mobile catalogs are split by area, sorted, in parity and fully re-exported by the barrels", () => {
  assert.ok(discoverPortions().length > 0, "no mobile catalog areas found");
  assert.deepEqual(auditCatalogs(), []);
});

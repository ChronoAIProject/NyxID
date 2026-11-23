import assert from "node:assert/strict";
import test from "node:test";
import { setAppTranslationLanguage } from "../../i18n/appTranslation";
import { billingMetricName } from "./billingMetric";

test("future metric identifiers retain a readable fallback, including prototype property names", (t) => {
  t.after(() => setAppTranslationLanguage("en"));
  for (const language of ["en", "zh-CN", "zh-TW"] as const) {
    setAppTranslationLanguage(language);
    assert.equal(billingMetricName("custom_usage_unit"), "custom usage unit");
    assert.equal(billingMetricName("constructor"), "constructor");
    assert.equal(billingMetricName("toString"), "toString");
  }
});

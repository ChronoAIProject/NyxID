import { describe, expect, it } from "vitest";
import { BILLING_METRICS, BILLING_UNITS, SERVICE_BILLING_METRICS, metricLabel } from "./billing-metrics";
import { billingMetricSchema } from "./billing";

describe("model tariff units", () => {
  it.each(["reasoning_tokens", "image_input_tokens", "image_output_tokens", "audio_input_tokens", "audio_output_tokens"] as const)("reads and labels %s without offering legacy pricing", (metric) => {
    expect(billingMetricSchema.parse(metric)).toBe(metric);
    expect(BILLING_METRICS).toContain(metric);
    expect(BILLING_UNITS[metric].tokenFamily).toBe(true);
    expect(metricLabel(metric)).toBe(metric.replaceAll("_", " "));
    expect(metricLabel(metric, 1)).toBe(metric.replaceAll("_", " ").slice(0, -1));
    expect(SERVICE_BILLING_METRICS).not.toContain(metric);
  });
});

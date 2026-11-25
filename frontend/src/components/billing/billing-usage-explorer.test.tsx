import { act, fireEvent, render } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { appI18n } from "@/i18n";
import { billingRow } from "@/test/billing-fixture";
import { dimensions, type Dimension } from "@/lib/billing-usage";
import { BillingUsageExplorer } from "./billing-usage-explorer";

vi.mock("./billing-quantity-chart", () => ({
  BillingQuantityChart: () => null,
}));
vi.mock("./billing-funding-chart", () => ({ BillingFundingChart: () => null }));
vi.mock("./billing-usage-details", () => ({
  ExpandableUsage: () => null,
  ServiceUsage: () => null,
  UsageMetricsDisclosure: () => null,
}));
vi.mock("@/components/billing-analytics/filter-picker", () => ({
  AnalyticsSelect: ({
    value,
    onChange,
    options,
  }: {
    value: string;
    onChange(value: string): void;
    options: { value: string; label: string }[];
  }) => (
    <select value={value} onChange={(e) => onChange(e.target.value)}>
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  ),
}));

afterEach(async () => {
  await appI18n.changeLanguage("en");
});
const plurals: Record<Dimension, string> = {
  service: "services",
  operation: "operations",
  model: "models",
  agent: "agents",
  layer: "billing layers",
  metric: "metrics",
};
for (const count of [1, 3]) {
  it(`renders every dimension at count ${count} in English and Chinese through the explorer`, async () => {
    const rows = Array.from({ length: count }, (_, i) =>
      billingRow({
        service_id: `id-${i}`,
        service_slug: `service-${i}`,
        operation: `op-${i}`,
        model: `model-${i}`,
        api_key_id: `agent-${i}`,
        layer: ["platform", "resale", "other"][i]!,
        metric: ["tokens", "requests", "bytes"][i]!,
      }),
    );
    const view = render(<BillingUsageExplorer catalog={[]} rows={rows} />);
    const countNode = () => view.container.querySelector(".usage-group-count")!;
    for (const dimension of Object.keys(dimensions) as Dimension[]) {
      await act(async () => {
        await appI18n.changeLanguage("en");
      });
      fireEvent.change(view.container.querySelector("select")!, {
        target: { value: dimension },
      });
      expect(countNode().textContent).toBe(
        `${count} ${count === 1 ? plurals[dimension].replace(/s$/, "") : plurals[dimension]}`,
      );
      for (const language of ["zh-CN", "zh-TW"]) {
        await act(async () => {
          await appI18n.changeLanguage(language);
        });
        expect(countNode().textContent).toMatch(
          new RegExp(`^${count} .*\\p{Script=Han}`, "u"),
        );
        expect(countNode().textContent).toContain(dimensions[dimension]);
        expect(countNode().textContent).not.toMatch(/[a-z]/i);
      }
    }
    view.unmount();
  });
}

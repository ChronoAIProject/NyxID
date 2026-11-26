import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it } from "vitest";
import { appI18n } from "@/i18n";
import { translateUi } from "@/i18n/use-ui-translation";
import { billingRow } from "@/test/billing-fixture";
import { TooltipProvider } from "@/components/ui/tooltip";
import { BillingQuantityChart } from "./billing-quantity-chart";

afterEach(async () => {
  await appI18n.changeLanguage("en");
});
it("localizes missing quantities in the accessible graph description when language changes", async () => {
  const view = render(
    <TooltipProvider>
      <BillingQuantityChart
        overview
        catalog={[]}
        rows={[
          billingRow({
            service_slug: "Service A",
            metric: "input_tokens",
            quantity: 10,
          }),
          billingRow({
            service_slug: "Service B",
            metric: "output_tokens",
            quantity: 5,
          }),
        ]}
      />
    </TooltipProvider>,
  );
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Filter metrics" }));
  await user.click(screen.getByRole("checkbox", { name: /output tokens/ }));
  await user.click(screen.getByRole("button", { name: "Done" }));
  const graph = screen.getByRole("img");
  const english = graph.getAttribute("aria-label");
  expect(english).toContain("Service B: input tokens not recorded");
  await act(async () => {
    await appI18n.changeLanguage("zh-CN");
  });
  expect(graph).toHaveAccessibleName(
    expect.stringContaining(translateUi("not recorded")),
  );
  expect(graph.getAttribute("aria-label")).not.toContain("not recorded");
  expect(graph.getAttribute("aria-label")).toContain("Service B");
  await act(async () => {
    await appI18n.changeLanguage("en");
  });
  expect(graph.getAttribute("aria-label")).toBe(english);
  view.unmount();
});

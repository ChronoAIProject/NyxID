import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { usageStats } from "@/test/admin-usage-fixture";
import { TokenMetricCaption, TokenMetricPicker } from "./token-metric-picker";
import { sumTokenMetrics, type TokenMetric } from "./token-metrics";

function PickerHarness() {
  const [selected, setSelected] = useState<TokenMetric[]>(["total_tokens"]);
  return (
    <>
      <TokenMetricPicker
        label="Summary token types"
        selected={selected}
        onChange={setSelected}
      />
      <output aria-label="Selected token sum">
        {sumTokenMetrics(usageStats(), selected)}
      </output>
      <TokenMetricCaption selected={selected} />
    </>
  );
}

it("adds selected token counts and labels multi-selections as custom", async () => {
  render(
    <TooltipProvider delayDuration={0}>
      <PickerHarness />
    </TooltipProvider>,
  );
  const user = userEvent.setup();
  const picker = screen.getByRole("button", { name: "Summary token types" });
  expect(picker).toHaveTextContent("Total tokens");
  expect(screen.getByLabelText("Selected token sum")).toHaveTextContent("120");

  await user.click(picker);
  await user.click(screen.getByRole("checkbox", { name: "Input tokens" }));
  expect(picker).toHaveTextContent("Custom (2)");
  expect(screen.getByLabelText("Selected token sum")).toHaveTextContent("120");
  expect(screen.getByText("Total tokens · Input tokens")).toBeInTheDocument();
  expect(
    screen.getByRole("button", {
      name: "How selected token counts are calculated",
    }).parentElement,
  ).toHaveTextContent("Total tokens · Input tokens");
  await user.hover(
    screen.getByRole("button", {
      name: "How selected token counts are calculated",
    }),
  );
  expect(await screen.findByRole("tooltip")).toHaveTextContent(
    "Selected input and output counts are shown but not added again.",
  );

  await user.click(screen.getByRole("checkbox", { name: "Output tokens" }));
  await user.click(screen.getByRole("checkbox", { name: "Cache-read tokens" }));
  await user.click(
    screen.getByRole("checkbox", { name: "Cache-write tokens" }),
  );
  expect(picker).toHaveTextContent("Custom (5)");
  expect(screen.getByLabelText("Selected token sum")).toHaveTextContent("155");

  await user.click(screen.getByRole("checkbox", { name: "Total tokens" }));
  expect(picker).toHaveTextContent("Custom (4)");
  expect(screen.getByLabelText("Selected token sum")).toHaveTextContent("155");
});

it("adds input and output when Total is absent, with cache counts added separately", () => {
  const usage = usageStats();
  expect(sumTokenMetrics(usage, ["prompt_tokens", "completion_tokens"])).toBe(
    120,
  );
  expect(sumTokenMetrics(usage, ["prompt_tokens", "cached_tokens"])).toBe(130);
  expect(
    sumTokenMetrics(usage, ["total_tokens", "prompt_tokens", "cached_tokens"]),
  ).toBe(150);
});

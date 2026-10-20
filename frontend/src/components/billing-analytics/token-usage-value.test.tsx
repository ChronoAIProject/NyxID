import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { usageStats } from "@/test/admin-usage-fixture";
import { TokenUsageValue } from "./token-usage-value";

it("includes image tokens in the total and explains the exact arithmetic on hover", async () => {
  const user = userEvent.setup();
  render(
    <TooltipProvider delayDuration={0}>
      <TokenUsageValue
        usage={usageStats({ image_input_tokens: 60, image_output_tokens: 15 })}
        selected={["total_tokens", "cached_tokens", "cache_creation_tokens"]}
      />
    </TooltipProvider>,
  );
  const value = screen.getByRole("button", { name: /Token count breakdown/ });
  expect(value).toHaveTextContent("120");
  await user.hover(value);
  const tooltip = await screen.findByRole("tooltip");
  expect(tooltip).toHaveTextContent("100 + 20 = 120");
  expect(tooltip).toHaveTextContent("Image input60");
  expect(tooltip).toHaveTextContent("Image output15");
  expect(tooltip).toHaveTextContent("Included in input / output");
  expect(tooltip).toHaveTextContent("Cache read 30 · cache write 5");
  expect(tooltip).toHaveTextContent("not added to the total");
});

it("opens for keyboard and tap and does not invent an image breakdown on older data", async () => {
  const user = userEvent.setup();
  render(
    <TooltipProvider delayDuration={0}>
      <TokenUsageValue usage={usageStats()} selected={["total_tokens"]} />
    </TooltipProvider>,
  );
  await user.tab();
  expect(await screen.findByRole("tooltip")).toHaveTextContent(
    "No separate image or voice breakdown was recorded",
  );
  await user.keyboard("{Escape}");
  await user.click(
    screen.getByRole("button", { name: /Token count breakdown/ }),
  );
  expect(await screen.findByRole("tooltip")).toHaveTextContent(
    "100 + 20 = 120",
  );
});

it("shows the selected-count arithmetic when total is not selected", async () => {
  const user = userEvent.setup();
  render(
    <TooltipProvider delayDuration={0}>
      <TokenUsageValue
        usage={usageStats({ image_input_tokens: 60, image_output_tokens: 15 })}
        selected={["prompt_tokens", "cached_tokens"]}
      />
    </TooltipProvider>,
  );
  await user.hover(
    screen.getByRole("button", { name: /Token count breakdown/ }),
  );
  const tooltip = await screen.findByRole("tooltip");
  expect(tooltip).toHaveTextContent("100 + 30 = 130");
  expect(tooltip).toHaveTextContent("Cache counts can overlap input");
});

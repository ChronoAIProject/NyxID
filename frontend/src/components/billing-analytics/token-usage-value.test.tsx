import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { usageStats } from "@/test/admin-usage-fixture";
import { TokenUsageValue } from "./token-usage-value";
import { appI18n } from "@/i18n";

it("updates an open token breakdown with the app language and explains older data in Chinese", async () => {
  const user = userEvent.setup();
  const view = render(
    <TooltipProvider delayDuration={0}>
      <TokenUsageValue
        usage={usageStats({ image_input_tokens: 60, image_output_tokens: 15, audio_input_tokens: 4, audio_output_tokens: 2 })}
        selected={["total_tokens"]}
      />
    </TooltipProvider>,
  );
  try {
    await user.hover(screen.getByRole("button"));
    await screen.findByRole("tooltip");
    await act(async () => { await appI18n.changeLanguage("zh-CN"); });
    const tooltip = screen.getByRole("tooltip");
    for (const label of ["图像输入", "图像输出", "语音输入", "语音输出"])
      expect(tooltip).toHaveTextContent(label);
    expect(tooltip).toHaveTextContent("100 + 20 = 120");
    expect(tooltip).not.toHaveTextContent(/Image input|Voice output/);
    view.rerender(
      <TooltipProvider delayDuration={0}>
        <TokenUsageValue usage={usageStats()} selected={["total_tokens"]} />
      </TooltipProvider>,
    );
    expect(tooltip).toHaveTextContent("所选范围内未记录单独的图像或语音用量明细。");
    await act(async () => { await appI18n.changeLanguage("en"); });
    expect(tooltip).toHaveTextContent("No separate image or voice breakdown was recorded for this selection.");
  } finally {
    view.unmount();
    await appI18n.changeLanguage("en");
  }
});

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

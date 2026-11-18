import { render as renderReact, screen, within } from "@testing-library/react";
import { expect, it } from "vitest";
import userEvent from "@testing-library/user-event";
import { TooltipProvider } from "@/components/ui/tooltip";
import { BillingAllowanceBenefits } from "./billing-allowance-benefits";
import {
  billingAllowance,
  billingCatalog,
  billingExpandedAllowances,
} from "@/test/billing-fixture";
import type { ReactNode } from "react";
const render = (node: ReactNode) =>
  renderReact(<TooltipProvider>{node}</TooltipProvider>);
it("groups allowances by display name, shows only consumed usage, and expands exact balances", async () => {
  const metrics = [
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "images",
  ] as const;
  const fixture = {
    data: { allowances: metrics.map((metric) => billingAllowance(metric)) },
  };
  render(
    <BillingAllowanceBenefits
      allowances={fixture.data.allowances}
      catalog={billingCatalog}
    />,
  );
  expect(screen.getByText("Example LLM")).toBeVisible();
  expect(screen.queryByText("example-llm")).not.toBeInTheDocument();
  expect(
    screen.getAllByText("Resets / expires", { selector: "dt" })[0],
  ).not.toBeVisible();
  const meter = screen.getByRole("meter", {
    name: "Example LLM free usage used",
  });
  expect(meter).toHaveAttribute(
    "aria-valuetext",
    "10% used; Input tokens 10% used; Output tokens 10% used; Cache-read tokens 10% used; Cache-write tokens 10% used; Images 10% used",
  );
  expect(
    screen.getByText("10% used", {
      selector: ".stacked-meter > .benefit-meter-caption",
    }),
  ).toBeInTheDocument();
  await userEvent.click(screen.getByText("Example LLM"));
  expect(
    screen.getAllByText("800").some((el) => el.closest("details")?.open),
  ).toBe(true);
});

it("selects multiple allowance metrics without hiding separate limits, exhaustion, or reservations", async () => {
  const fixture = {
    data: { allowances: billingExpandedAllowances() },
  };
  render(
    <BillingAllowanceBenefits
      allowances={fixture.data.allowances}
      catalog={billingCatalog}
    />,
  );
  await userEvent.click(screen.getByText("Example LLM"));
  const inspector = screen.getByRole("region", {
    name: "Selected allowance balances",
  });
  expect(within(inspector).getByText("input tokens")).toBeVisible();
  expect(within(inspector).getAllByText("Limit")).toHaveLength(2);
  expect(
    within(inspector).getAllByText("1,000,000", { selector: "dd" }),
  ).toHaveLength(2);
  expect(within(inspector).getByText("100,000,000")).toBeVisible();
  expect(within(inspector).getByText("95,250,308")).toBeVisible();
  expect(within(inspector).getByText("536,202")).toBeVisible();
  expect(
    within(inspector).getByRole("meter", {
      name: "input tokens allowance 1 used",
    }),
  ).toHaveAttribute("aria-valuenow", "100");
  expect(
    within(inspector).getByRole("meter", {
      name: "input tokens allowance 2 used",
    }),
  ).toHaveAttribute("aria-valuenow", "4.21349");
  expect(
    within(inspector).getByRole("meter", {
      name: "input tokens allowance 2 used",
    }),
  ).toHaveAttribute("aria-valuetext", "4.21% used; 536,202 held");
  expect(within(inspector).getAllByText("Resets / expires")).toHaveLength(1);
  expect(
    within(inspector).queryByText("output tokens"),
  ).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Filter metrics" }));
  const picker = within(screen.getByRole("dialog"));
  const output = picker.getByRole("checkbox", { name: /^output tokens/ });
  await userEvent.click(output);
  expect(output).toBeChecked();
  expect(within(inspector).getByText("output tokens")).toBeVisible();
  await userEvent.click(picker.getByRole("button", { name: "Done" }));
  expect(within(inspector).getByText("output tokens")).toBeVisible();
  expect(within(inspector).getByText("input tokens")).toBeVisible();
  expect(within(inspector).getAllByText("Limit")).toHaveLength(4);
  await userEvent.click(screen.getByRole("button", { name: "Filter metrics" }));
  await userEvent.click(
    within(screen.getByRole("dialog")).getByRole("checkbox", {
      name: /^images/,
    }),
  );
  await userEvent.click(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "Done",
    }),
  );
  expect(within(inspector).getByText("images")).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", { name: "Remove metric: images" }),
  );
  expect(within(inspector).queryByText("images")).not.toBeInTheDocument();
});

it("keeps different allowance windows separate and pages large groups", async () => {
  const base = billingAllowance();
  const fixture = {
    data: {
      allowances: [
        ...Array.from({ length: 7 }, (_, index) => ({
          ...base,
          allowance: { ...base.allowance, id: `daily-${index}` },
        })),
        {
          ...base,
          period_end: null,
          allowance: {
            ...base.allowance,
            id: "one-time",
            recurrence: "one_time" as const,
          },
        },
      ],
    },
  };
  render(
    <BillingAllowanceBenefits
      allowances={fixture.data.allowances}
      catalog={billingCatalog}
    />,
  );
  await userEvent.click(screen.getByText("Example LLM"));
  const inspector = screen.getByRole("region", {
    name: "Selected allowance balances",
  });
  expect(within(inspector).getAllByText("Limit")).toHaveLength(5);
  await userEvent.click(
    screen.getByRole("button", { name: "Next allowances" }),
  );
  expect(within(inspector).getAllByText("Limit")).toHaveLength(2);
  expect(within(inspector).getByText("Allowance 6")).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "Filter metrics" }));
  const picker = within(screen.getByRole("dialog"));
  await userEvent.click(picker.getByRole("button", { name: "Clear" }));
  await userEvent.click(
    picker.getByRole("checkbox", { name: /^input tokens · one time/ }),
  );
  await userEvent.click(picker.getByRole("button", { name: "Done" }));
  expect(within(inspector).getAllByText("Limit")).toHaveLength(1);
  expect(within(inspector).getByText("No reset or expiry")).toBeVisible();
  expect(within(inspector).queryByText("7,000")).not.toBeInTheDocument();
});

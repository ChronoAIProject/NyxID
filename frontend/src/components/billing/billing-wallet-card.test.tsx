import "@/i18n";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { billingWallet } from "@/test/billing-fixture";
import { BillingWalletActions } from "./billing-wallet-card";

function actions(overrides: Partial<React.ComponentProps<typeof BillingWalletActions>> = {}) {
  return (
    <TooltipProvider>
      <BillingWalletActions
        wallet={billingWallet()}
        loading={false}
        unavailable={false}
        error={null}
        onRetry={vi.fn()}
        onProvision={vi.fn()}
        provisioning={false}
        billingReady
        onTopUp={vi.fn().mockResolvedValue(undefined)}
        topUpPending={false}
        {...overrides}
      />
    </TooltipProvider>
  );
}

describe("Add credits for the USD 1 whole-credit wallet", () => {
  it("selects US$20 and sends the exact whole-credit quantity for each dollar preset", async () => {
    const user = userEvent.setup();
    const onTopUp = vi.fn().mockResolvedValue(undefined);
    render(actions({ onTopUp }));
    await user.click(screen.getByRole("button", { name: "Add credits" }));
    const dialog = within(screen.getByRole("dialog"));
    const input = dialog.getByRole("spinbutton", { name: "Or enter credits" });
    expect(input).toHaveValue(20);
    expect(dialog.getByRole("button", { name: "US$20" })).toHaveAttribute("aria-pressed", "true");
    expect(dialog.getByText(/Minimum purchase: 10 credits/)).toBeVisible();
    for (const amount of [20, 50, 100, 500]) {
      await user.click(dialog.getByRole("button", { name: `US$${amount}` }));
      expect(input).toHaveValue(amount);
      expect(dialog.getByText(`${amount} credits · Subtotal: USD ${amount}.00`)).toBeVisible();
      await user.click(dialog.getByRole("button", { name: "Continue to payment" }));
      expect(onTopUp).toHaveBeenLastCalledWith(amount);
    }
  });

  it("enforces minimum, custom whole credits, maximum and overflow before submitting", async () => {
    const user = userEvent.setup();
    const onTopUp = vi.fn().mockResolvedValue(undefined);
    render(actions({ onTopUp, topUpOpen: true }));
    const dialog = within(screen.getByRole("dialog"));
    const input = dialog.getByRole("spinbutton");
    const submit = dialog.getByRole("button", { name: "Continue to payment" });
    expect(input).toHaveAttribute("min", "10");
    expect(input).toHaveAttribute("max", "10000000");
    for (const [value, valid] of [
      ["9", false], ["10", true], ["11", true], ["20", true],
      ["50", true], ["100", true], ["500", true], ["10000000", true],
      ["10000001", false], ["9223372036854775808", false],
      ["1e309", false], ["10.5", false], ["-1", false], ["", false],
    ] as const) {
      fireEvent.change(input, { target: { value } });
      if (valid) {
        expect(submit, value).toBeEnabled();
        await user.click(submit);
        expect(onTopUp).toHaveBeenLastCalledWith(Number(value));
      } else {
        expect(submit, value).toBeDisabled();
        const calls = onTopUp.mock.calls.length;
        await user.click(submit);
        expect(onTopUp).toHaveBeenCalledTimes(calls);
      }
    }
  });

  it("resets after cancel/reopen and external opening while preserving pending edits", async () => {
    const user = userEvent.setup();
    const view = render(actions());
    await user.click(screen.getByRole("button", { name: "Add credits" }));
    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "37" } });
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(screen.getByRole("button", { name: "Add credits" }));
    expect(screen.getByRole("spinbutton")).toHaveValue(20);
    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "51" } });
    view.rerender(actions({ topUpPending: true }));
    expect(screen.getByRole("spinbutton")).toHaveValue(51);
    expect(screen.getByRole("spinbutton")).toBeDisabled();
    view.rerender(actions({ topUpPending: false }));
    expect(screen.getByRole("spinbutton")).toHaveValue(51);
    expect(screen.getByRole("spinbutton")).toBeEnabled();
    view.rerender(actions({ topUpOpen: false }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    view.rerender(actions({ topUpOpen: true }));
    expect(screen.getByRole("spinbutton")).toHaveValue(20);
  });
});

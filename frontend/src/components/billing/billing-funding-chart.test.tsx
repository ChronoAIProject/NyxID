import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { billingRow } from "@/test/billing-fixture";
import { BillingFundingChart } from "./billing-funding-chart";

describe("BillingFundingChart", () => {
  it("uses exact fractional funding rather than truncated legacy micros", () => {
    render(
      <BillingFundingChart
        rows={[
          billingRow({
            estimated_credits: "0.00000000001",
            grant_credits: "0.000000000005",
            allowance_credits: "0.000000000003",
            wallet_credits: "0.000000000002",
            estimated_credits_micros: 0,
            grant_credits_micros: 0,
            allowance_credits_micros: 0,
            wallet_credits_micros: 0,
          }),
        ]}
      />,
    );
    expect(screen.getByRole("img")).toHaveAccessibleName(
      "Funding: Credit grants: <0.000001 credits (50%), Allowances: <0.000001 credits (30%), Wallet: <0.000001 credits (20%)",
    );
    expect(screen.queryByText(/unavailable/)).not.toBeInTheDocument();
  });

  it("labels incomplete composition as known funding and keeps unknown values unavailable", () => {
    render(
      <BillingFundingChart
        rows={[
          billingRow({
            estimated_credits: null,
            allowance_credits: null,
            wallet_credits: null,
            grant_credits: "1.5",
          }),
        ]}
      />,
    );
    expect(screen.getByRole("img")).toHaveAccessibleName(
      "Known funding: Credit grants: 1.5 credits (100%)",
    );
    expect(
      screen.getAllByText(
        (_, element) =>
          element?.tagName === "STRONG" &&
          element.textContent === "Unavailable credits",
      ),
    ).toHaveLength(3);
    expect(screen.getByText(/bar shows only known funding/)).toBeVisible();
  });

  it("represents free usage without implying a funding share", () => {
    render(
      <BillingFundingChart
        rows={[
          billingRow({
            billable: false,
            estimated_credits_micros: 0,
            grant_credits_micros: 0,
            allowance_credits_micros: 0,
            wallet_credits_micros: 0,
          }),
        ]}
      />,
    );
    expect(screen.getByRole("img")).toHaveAccessibleName(
      "Funding: No known funding values for this period",
    );
    expect(screen.getAllByText("0 credits")).toHaveLength(3);
    expect(screen.queryByText(/unavailable/)).not.toBeInTheDocument();
  });
});

import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { BillingBenefits } from "./billing-benefits";

const mocks = vi.hoisted(() => ({
  grants: vi.fn(),
  allowances: vi.fn(),
}));

vi.mock("@/hooks/use-billing-credits", () => ({
  useActiveCreditGrants: mocks.grants,
  useCurrentAllowances: mocks.allowances,
}));

function apiError(status: number, message: string) {
  return new ApiError(status, {
    error: "billing_error",
    error_code: status,
    message,
  });
}

function query(error: unknown) {
  return {
    data: undefined,
    error,
    isLoading: false,
    refetch: vi.fn(),
  };
}

beforeEach(() => vi.clearAllMocks());

describe("BillingBenefits", () => {
  it("hides the rollout-gated section only when both reads return 403", () => {
    mocks.grants.mockReturnValue(query(apiError(403, "grants hidden")));
    mocks.allowances.mockReturnValue(query(apiError(403, "allowances hidden")));

    const { container } = render(<BillingBenefits />);

    expect(container).toBeEmptyDOMElement();
  });

  it("surfaces a real error when the other benefit read is rollout-hidden", () => {
    mocks.grants.mockReturnValue(query(apiError(403, "grants hidden")));
    mocks.allowances.mockReturnValue(
      query(apiError(503, "Allowances are temporarily unavailable")),
    );

    render(<BillingBenefits />);

    expect(
      screen.getByText("Allowances are temporarily unavailable"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });
});

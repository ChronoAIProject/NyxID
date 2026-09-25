import { render as renderReact, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { BillingBenefits } from "./billing-benefits";
import { TooltipProvider } from "@/components/ui/tooltip";
import {
  billingGrant,
  billingAllowance,
  billingCatalog,
} from "@/test/billing-fixture";
import type { ReactNode } from "react";
import userEvent from "@testing-library/user-event";
const render = (node: ReactNode) =>
  renderReact(<TooltipProvider>{node}</TooltipProvider>);

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

it.each(["org_members", "groups"] as const)(
  "renders personal balances received through %s",
  (kind) => {
    const targets = {
      target_kind: kind,
      target_org_ids: kind === "org_members" ? ["org"] : [],
      target_group_ids: kind === "groups" ? ["group"] : [],
    };
    mocks.grants.mockReturnValue({
      ...query(null),
      data: {
        grants: [
          {
            ...billingGrant({ reserved_micros: 0 }),
            ...targets,
            id: "grant",
            scope: { all_services: true },
            remaining_micros: 2_000_000,
          },
        ],
      },
    });
    mocks.allowances.mockReturnValue({
      ...query(null),
      data: {
        allowances: [
          {
            ...billingAllowance("requests"),
            allowance: {
              ...billingAllowance("requests").allowance,
              ...targets,
              id: "allowance",
              service_slug: "service",
              metric: "requests",
            },
            remaining_quantity: 80,
          },
        ],
      },
    });
    render(<BillingBenefits />);
    expect(
      screen.getByText("2", {
        exact: false,
        selector: ".compact-grant-balance > strong",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText("80 requests")).toBeInTheDocument();
  },
);

it("groups allowances by display name, shows only consumed usage, and expands exact balances", async () => {
  const metrics = [
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "images",
  ] as const;
  mocks.grants.mockReturnValue({
    ...query(null),
    data: { grants: [billingGrant()] },
  });
  mocks.allowances.mockReturnValue({
    ...query(null),
    data: { allowances: metrics.map((metric) => billingAllowance(metric)) },
  });
  render(<BillingBenefits catalog={billingCatalog} />);
  expect(screen.getByText("Example LLM")).toBeVisible();
  expect(screen.queryByText("example-llm")).not.toBeInTheDocument();
  expect(screen.getByText("Expires")).not.toBeVisible();
  expect(
    screen.getAllByText("Resets / expires", { selector: "dt" })[0],
  ).not.toBeVisible();
  const chart = screen.getByRole("img", { name: /Allowance usage/ });
  await userEvent.hover(chart);
  const tooltip = await screen.findByRole("tooltip");
  expect(tooltip.querySelectorAll("dl > div")).toHaveLength(5);
  expect(tooltip.querySelectorAll("dd")).toHaveLength(5);
  expect(
    [...tooltip.querySelectorAll("dd")].every((el) => el.textContent === "10%"),
  ).toBe(true);
  await userEvent.keyboard("{Escape}");
  await userEvent.click(screen.getByText("Example LLM"));
  expect(
    screen.getAllByText("800").some((el) => el.closest("details")?.open),
  ).toBe(true);
});

import { render as renderReact, screen, within } from "@testing-library/react";
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
    expect(screen.getByText("2 of 10 credits left")).toBeInTheDocument();
    expect(screen.getByText("80 of 1K left")).toBeInTheDocument();
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
  // One meter per allowance, each with its own percentage.
  const meters = screen.getAllByRole("meter", { name: /^Example LLM / });
  expect(meters).toHaveLength(5);
  expect(
    meters.every(
      (meter) => meter.getAttribute("aria-valuetext") === "10% used",
    ),
  ).toBe(true);
  expect(screen.getByText("Cache-read tokens")).toBeVisible();
  await userEvent.click(screen.getByText("Example LLM"));
  expect(
    screen.getAllByText("800").some((el) => el.closest("details")?.open),
  ).toBe(true);
});

it("shows one meter tile per allowance with thresholds, an unused state and overflow", async () => {
  const allowance = (
    metric: Parameters<typeof billingAllowance>[0],
    consumed: number,
  ) =>
    billingAllowance(metric, {
      consumed_quantity: consumed,
      reserved_quantity: 0,
      remaining_quantity: 1000 - consumed,
    });
  mocks.grants.mockReturnValue({ ...query(null), data: { grants: [] } });
  mocks.allowances.mockReturnValue({
    ...query(null),
    data: {
      allowances: [
        allowance("input_tokens", 412),
        allowance("output_tokens", 800),
        allowance("cache_read_tokens", 1000),
        allowance("cache_write_tokens", 0),
        allowance("images", 10),
        allowance("requests", 20),
        allowance("bytes", 30),
        allowance("tokens", 40),
      ],
    },
  });
  render(<BillingBenefits catalog={billingCatalog} />);
  const tile = (label: string) =>
    screen
      .getByText(label, { selector: ".benefit-tile-label" })
      .closest(".benefit-tile") as HTMLElement;
  // metricRank order; the first six are visible, the rest behind Details.
  expect(
    [...document.querySelectorAll(".benefit-tile-label")].map(
      (el) => el.textContent,
    ),
  ).toEqual([
    "Tokens",
    "Input tokens",
    "Output tokens",
    "Cache-read tokens",
    "Cache-write tokens",
    "Images",
  ]);
  expect(screen.getByText("+2 more")).toBeInTheDocument();
  expect(within(tile("Input tokens")).getByText("41.2%")).toBeVisible();
  expect(within(tile("Input tokens")).getByRole("meter")).toHaveAttribute(
    "aria-valuetext",
    "41.2% used",
  );
  expect(tile("Input tokens")).not.toHaveClass("is-warning");
  expect(tile("Output tokens")).toHaveClass("is-warning");
  expect(tile("Cache-read tokens")).toHaveClass("is-exhausted");
  expect(tile("Cache-read tokens")).not.toHaveClass("is-warning");
  expect(
    within(tile("Cache-write tokens")).getByText("Unused · 1K left"),
  ).toBeVisible();
  expect(
    within(tile("Input tokens")).getByText("588 of 1K left"),
  ).toBeVisible();
});

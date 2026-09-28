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
    expect(
      screen.getByText("2", {
        exact: false,
        selector: ".compact-grant-balance > strong",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText("80 left")).toBeInTheDocument();
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

it("stacks allowances into one bar whose segments and legend swatches match", () => {
  const allowance = (
    metric: Parameters<typeof billingAllowance>[0],
    consumed: number,
  ) =>
    billingAllowance(metric, {
      consumed_quantity: consumed,
      reserved_quantity: 0,
      remaining_quantity: 1000 - consumed,
    });
  mocks.grants.mockReturnValue({
    ...query(null),
    data: {
      grants: [
        billingGrant({
          id: "a",
          amount_micros: 10_000_000,
          remaining_micros: 5_000_000,
          reserved_micros: 0,
        }),
        billingGrant({
          id: "b",
          reason: "Promo",
          amount_micros: 30_000_000,
          remaining_micros: 27_000_000,
          reserved_micros: 0,
        }),
      ],
    },
  });
  mocks.allowances.mockReturnValue({
    ...query(null),
    data: {
      allowances: [
        allowance("input_tokens", 412),
        allowance("output_tokens", 244),
        allowance("cache_read_tokens", 0),
        allowance("images", 240),
        allowance("requests", 1000),
      ],
    },
  });
  render(<BillingBenefits catalog={billingCatalog} />);

  // Free usage: equal shares, contiguous from the left, total = average.
  const service = screen.getByText("Example LLM").closest("details")!;
  const segments = [...service.querySelectorAll<HTMLElement>(".stack-seg")];
  expect(segments.map((segment) => segment.style.width)).toEqual([
    "8.24%",
    "4.88%",
    "4.8%",
    "20%",
  ]);
  expect(
    segments.map((segment) =>
      [...segment.classList].find((name) => name.startsWith("stack-step-")),
    ),
  ).toEqual(["stack-step-0", "stack-step-1", "stack-step-3", "stack-step-4"]);
  expect(within(service).getByText("38% used")).toBeInTheDocument();
  // Every legend item (unused ones included) carries its segment's swatch.
  const legend = within(service).getByRole("list", {
    name: "Remaining free usage",
  });
  const items = within(legend).getAllByRole("listitem");
  expect(items.map((item) => item.textContent)).toEqual([
    "Input588 left",
    "Output756 left",
    "Cache read1K left",
    "Images760 left",
    "Requests0 left(used up)",
  ]);
  items.forEach((item, step) =>
    expect(item.querySelector(".stack-swatch")).toHaveClass(
      `stack-step-${step}`,
    ),
  );
  // One spent allowance is flagged even though the average is low.
  expect(service).not.toHaveClass("stack-warning");
  expect(service).not.toHaveClass("stack-exhausted");
  expect(items[4]).toHaveClass("is-exhausted");
  // Details rows carry the same swatches.
  const rows = [...service.querySelectorAll(".benefit-unit .stack-swatch")];
  expect(rows.map((swatch) => swatch.className)).toEqual(
    [0, 1, 2, 3, 4].map((step) => `stack-swatch stack-step-${step}`),
  );

  // Grants: proportional to consumed credits over all original credits.
  const grants = screen.getByText("Credit grants").closest("details")!;
  expect(
    [...grants.querySelectorAll<HTMLElement>(".stack-seg")].map(
      (segment) => segment.style.width,
    ),
  ).toEqual(["12.5%", "7.5%"]);
  expect(within(grants).getByText("20% used")).toBeInTheDocument();
  expect(
    [...grants.querySelectorAll("h4 .stack-swatch")].map(
      (swatch) => swatch.className,
    ),
  ).toEqual(["stack-swatch stack-step-0", "stack-swatch stack-step-1"]);
});

it("switches the stack to warning at an 80% average and destructive when spent", () => {
  mocks.grants.mockReturnValue({ ...query(null), data: { grants: [] } });
  mocks.allowances.mockReturnValue({
    ...query(null),
    data: {
      allowances: [
        billingAllowance("input_tokens", {
          consumed_quantity: 900,
          reserved_quantity: 0,
          remaining_quantity: 100,
        }),
        billingAllowance("output_tokens", {
          consumed_quantity: 800,
          reserved_quantity: 0,
          remaining_quantity: 200,
        }),
      ],
    },
  });
  const view = render(<BillingBenefits catalog={billingCatalog} />);
  expect(screen.getByText("Example LLM").closest("details")).toHaveClass(
    "stack-warning",
  );
  view.unmount();
  mocks.allowances.mockReturnValue({
    ...query(null),
    data: {
      allowances: [
        billingAllowance("requests", {
          consumed_quantity: 1000,
          reserved_quantity: 0,
          remaining_quantity: 0,
        }),
      ],
    },
  });
  render(<BillingBenefits catalog={billingCatalog} />);
  expect(screen.getByText("Example LLM").closest("details")).toHaveClass(
    "stack-exhausted",
  );
  expect(
    screen.getByText("100% used", {
      selector: ".stacked-meter > .benefit-meter-caption",
    }),
  ).toBeInTheDocument();
});

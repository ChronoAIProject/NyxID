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
  // Width is min + (100% - k * min) * share; assert the calc inputs.
  const calcInputs = (segment: HTMLElement) => [
    segment.style.getPropertyValue("--stack-share"),
    segment.style.getPropertyValue("--stack-k"),
  ];
  expect(segments.map(calcInputs)).toEqual([
    ["0.0824", "4"],
    ["0.0488", "4"],
    ["0.048", "4"],
    ["0.2", "4"],
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
    [...grants.querySelectorAll<HTMLElement>(".stack-seg")].map(calcInputs),
  ).toEqual([
    ["0.125", "2"],
    ["0.075", "2"],
  ]);
  expect(within(grants).getByText("20% used")).toBeInTheDocument();
  // The Details table carries each tranche's swatch.
  expect(
    [...grants.querySelectorAll(".grants-desktop tbody .stack-swatch")].map(
      (swatch) => swatch.className,
    ),
  ).toEqual(["stack-swatch stack-step-0", "stack-swatch stack-step-1"]);
});

it("switches the stack to warning in the last 5% and destructive when spent", () => {
  mocks.grants.mockReturnValue({ ...query(null), data: { grants: [] } });
  mocks.allowances.mockReturnValue({
    ...query(null),
    data: {
      allowances: [
        billingAllowance("input_tokens", {
          consumed_quantity: 960,
          reserved_quantity: 0,
          remaining_quantity: 40,
        }),
        billingAllowance("output_tokens", {
          consumed_quantity: 950,
          reserved_quantity: 0,
          remaining_quantity: 50,
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

it("merges every grant into one row with a soonest-expiring table", async () => {
  const now = Date.parse("2026-09-28T12:00:00Z");
  vi.useFakeTimers({ now, toFake: ["Date"] });
  try {
    mocks.allowances.mockReturnValue({
      ...query(null),
      data: { allowances: [] },
    });
    mocks.grants.mockReturnValue({
      ...query(null),
      data: {
        grants: [
          billingGrant({
            id: "later",
            reason: "Scoped tranche",
            amount_micros: 10_000_000,
            remaining_micros: 6_000_000,
            reserved_micros: 500_000,
            scope: {
              all_services: false,
              service_ids: ["a", "b", "c"],
              service_slugs: ["example-llm", "free-service", "unused-service"],
            },
            expires_at: "2026-12-01T12:00:00Z",
          }),
          billingGrant({
            id: "soon",
            reason: "Welcome credits",
            amount_micros: 5_000_000,
            remaining_micros: 4_800_000,
            reserved_micros: 0,
            expires_at: "2026-10-03T12:00:00Z",
          }),
          billingGrant({
            id: "open",
            reason: "Referral bonus",
            amount_micros: 1_000_000,
            remaining_micros: 1_000_000,
            reserved_micros: 0,
            expires_at: null,
            activation_state: "pending_activation",
          }),
          billingGrant({
            id: "spent-soon",
            reason: "Nearly spent",
            amount_micros: 1_000_000,
            remaining_micros: 40_000,
            reserved_micros: 0,
            expires_at: "2026-11-01T12:00:00Z",
          }),
        ],
      },
    });
    // Expiry dates render in the viewer's timezone.
    const day = (iso: string) =>
      new Date(iso).toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
      });
    render(<BillingBenefits catalog={billingCatalog} />);
    // One "Credit grants" row, whatever the scopes.
    expect(
      screen.getAllByText("Credit grants", { selector: "strong" }),
    ).toHaveLength(1);
    const row = screen
      .getByText("Credit grants", { selector: "strong" })
      .closest("details")!;
    expect(within(row).getByText("4 grants")).toBeInTheDocument();
    // Available = sum(remaining - reserved) over spendable tranches =
    // 10.84 - 0.5; the pending 1 credit is not spendable yet.
    expect(within(row).getByText("10.34 credits")).toBeInTheDocument();
    expect(within(row).getByText("· 1 pending")).toBeInTheDocument();
    // Segments in consumption order (soonest expiry first), sized by credits.
    expect(
      [...row.querySelectorAll<HTMLElement>("summary .stack-seg")].map(
        (segment) => segment.dataset.key,
      ),
    ).toEqual(["soon", "spent-soon", "later"]);
    const table = row.querySelector<HTMLElement>(".grants-desktop")!;
    const rows = within(table).getAllByRole("row").slice(1);
    expect(
      rows.map((tableRow) =>
        within(tableRow)
          .getAllByRole("cell")
          .map((cell) => cell.textContent),
      ),
    ).toEqual([
      [
        "Welcome credits",
        "All services",
        "4.8 of 5",
        "4%",
        `${day("2026-10-03T12:00:00Z")} · in 5 days`,
      ],
      [
        "Nearly spent",
        "All services",
        "0.04 of 1",
        "96%",
        day("2026-11-01T12:00:00Z"),
      ],
      [
        "Scoped tranche",
        "Example LLM, Free service +1 more",
        "6 of 100.5 reserved",
        "40%",
        day("2026-12-01T12:00:00Z"),
      ],
      ["Referral bonusPending", "All services", "1 of 1", "0%", "No expiry"],
    ]);
    // Only a non-normal state gets a badge; the last-5% row is flagged.
    expect(within(table).getAllByText("Pending")).toHaveLength(1);
    // A pending tranche has no bar segment, so its swatch is hollow.
    expect(rows[3]!.querySelector(".stack-swatch")).toHaveClass(
      "stack-swatch-empty",
    );
    expect(rows[0]!.querySelector(".stack-swatch")).not.toHaveClass(
      "stack-swatch-empty",
    );
    expect(within(rows[1]!).getByText("96%")).toHaveClass("text-warning");
    expect(within(rows[0]!).getByText("4%")).not.toHaveClass("text-warning");
    // The two-line phone rows carry the same tranches.
    expect(
      within(row.querySelector<HTMLElement>(".grants-mobile")!).getAllByRole(
        "listitem",
      ),
    ).toHaveLength(4);
  } finally {
    vi.useRealTimers();
  }
});

it("keeps an exact sub-micro grant balance visible", () => {
  mocks.grants.mockReturnValue({
    ...query(null),
    data: {
      grants: [
        billingGrant({
          amount: "1",
          remaining: "0.0000008",
          reserved: "0",
          remaining_micros: 0,
          reserved_micros: 0,
        }),
      ],
    },
  });
  mocks.allowances.mockReturnValue({
    ...query(null),
    data: { allowances: [] },
  });
  render(<BillingBenefits />);
  expect(
    screen.getByText("<0.01 credits", {
      selector: ".compact-grant-balance > strong",
    }),
  ).toBeVisible();
});

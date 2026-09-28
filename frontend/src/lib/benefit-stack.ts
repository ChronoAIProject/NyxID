import { number } from "@/lib/billing-display";

/** Distinct opacity steps; a longer stack merges its tail into the last one. */
export const STACK_STEPS = 5;

export type StackStatus = "normal" | "warning" | "exhausted";

export interface StackInput {
  readonly key: string;
  /** Full name for tooltips and screen readers, e.g. "Input tokens". */
  readonly label: string;
  /** Legend name, e.g. "Input". */
  readonly short: string;
  readonly used: number;
  readonly limit: number;
  /** Legend value, e.g. "580K left". */
  readonly legend: string;
  /** Tooltip detail, e.g. "580K of 1M left". */
  readonly detail: string;
}

export interface StackItem extends StackInput {
  readonly step: number;
  /** This input's own utilization, 0..100. */
  readonly percent: number;
  readonly status: StackStatus;
}

/** One contiguous segment of the bar and one legend entry. */
export interface StackEntry {
  readonly key: string;
  readonly label: string;
  readonly short: string;
  readonly legend: string;
  readonly step: number;
  /** Width as a percentage of the whole bar. */
  readonly share: number;
  readonly status: StackStatus;
  /** Tooltip text. */
  readonly tooltip: string;
  /** Number of inputs merged into this entry (tail only). */
  readonly merged: number;
}

export interface Stack {
  readonly entries: readonly StackEntry[];
  readonly items: readonly StackItem[];
  /** Total fill, 0..100 (the exact figure for captions). */
  readonly overall: number;
  readonly status: StackStatus;
  /**
   * Visible segments: entries with a non-zero share. Each renders as
   * `min + (100% - k * min) * share`, so small values still read as pills
   * and the bar only fills completely when everything is used.
   */
  readonly k: number;
}

const clamp = (value: number) => Math.min(1, Math.max(0, value));

export function stackStatus(percent: number): StackStatus {
  return percent >= 100 ? "exhausted" : percent >= 80 ? "warning" : "normal";
}

/** Percentages with up to two decimals, e.g. "41.2", "0.71", "<0.01". */
export function formatPercent(percent: number): string {
  return percent > 0 && percent < 0.01
    ? "<0.01"
    : number(Math.round(percent * 100) / 100);
}

/**
 * The overall caption: whole numbers from 10%, e.g. "18% used", but never
 * rounded up to a spent-looking 100%.
 */
export function overallCaption(percent: number): string {
  if (percent <= 0) return "Unused";
  const whole = Math.round(percent);
  const text =
    percent >= 10 && (whole < 100 || percent >= 100)
      ? number(whole)
      : formatPercent(percent);
  return `${text}% used`;
}

function build(
  inputs: readonly StackInput[],
  shares: readonly number[],
): Stack {
  const items = inputs.map((input, index) => {
    const percent = input.limit > 0 ? clamp(input.used / input.limit) * 100 : 0;
    return {
      ...input,
      step: Math.min(index, STACK_STEPS - 1),
      percent,
      status: stackStatus(percent),
    };
  });
  const entry = (item: StackItem, index: number): StackEntry => ({
    key: item.key,
    label: item.label,
    short: item.short,
    legend: item.legend,
    step: item.step,
    share: shares[index]!,
    status: item.status,
    tooltip: `${item.label} · ${formatPercent(item.percent)}% used · ${item.detail}`,
    merged: 1,
  });
  let entries: StackEntry[];
  if (items.length > STACK_STEPS) {
    const head = items.slice(0, STACK_STEPS - 1);
    const tail = items.slice(STACK_STEPS - 1);
    const tailShares = shares.slice(STACK_STEPS - 1);
    const worst = tail.some((item) => item.status === "exhausted")
      ? "exhausted"
      : tail.some((item) => item.status === "warning")
        ? "warning"
        : "normal";
    entries = [
      ...head.map(entry),
      {
        key: "more",
        label: `${tail.length} more`,
        short: `${tail.length} more`,
        legend: "",
        step: STACK_STEPS - 1,
        share: tailShares.reduce((sum, share) => sum + share, 0),
        status: worst,
        tooltip: tail
          .map(
            (item) =>
              `${item.label} · ${formatPercent(item.percent)}% used · ${item.detail}`,
          )
          .join("\n"),
        merged: tail.length,
      },
    ];
  } else {
    entries = items.map(entry);
  }
  // Rounded so N equal shares that are all spent sum to exactly 100.
  const overall = Math.min(
    100,
    Math.round(shares.reduce((sum, share) => sum + share, 0) * 1e6) / 1e6,
  );
  return {
    entries,
    items,
    overall,
    status: stackStatus(overall),
    k: entries.filter((entry) => entry.share > 0).length,
  };
}

/**
 * Free-usage allowances: each of N owns an equal 1/N of the bar and fills
 * its share by its own utilization, so the total fill is the average.
 */
export function equalShareStack(inputs: readonly StackInput[]): Stack {
  const count = inputs.length;
  return build(
    inputs,
    inputs.map((input) =>
      count && input.limit > 0
        ? (clamp(input.used / input.limit) / count) * 100
        : 0,
    ),
  );
}

/**
 * Credit grants: each grant's consumed credits as a share of all the grants'
 * original credits, so the total fill is the row's overall utilization.
 */
export function proportionalStack(inputs: readonly StackInput[]): Stack {
  const total = inputs.reduce((sum, input) => sum + input.limit, 0);
  return build(
    inputs,
    inputs.map((input) =>
      total > 0
        ? (Math.min(Math.max(0, input.used), input.limit) / total) * 100
        : 0,
    ),
  );
}

/** Screen-reader breakdown: overall, then every input's own utilization. */
export function stackValueText(stack: Stack): string {
  return [
    overallCaption(stack.overall),
    ...stack.items.map(
      (item) => `${item.label} ${formatPercent(item.percent)}% used`,
    ),
  ].join("; ");
}

/** Base color for a stack: neutral, warning at 80% used, destructive when spent. */
export function stackStatusClass(status: StackStatus) {
  return status === "exhausted"
    ? "stack-exhausted"
    : status === "warning"
      ? "stack-warning"
      : undefined;
}

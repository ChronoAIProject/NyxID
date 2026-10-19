import { creditsLabel, knownTotal } from "@/lib/billing-display";
import type { BillingUsageRow } from "@/schemas/billing";

const lanes = [
  {
    key: "grant_credits_micros",
    label: "Credit grants",
    color: "var(--color-success)",
  },
  {
    key: "allowance_credits_micros",
    label: "Allowances",
    color: "var(--color-warning)",
  },
  {
    key: "wallet_credits_micros",
    label: "Wallet",
    color: "var(--color-info)",
  },
] as const;

function percentage(value: bigint, total: bigint) {
  if (total <= 0n || value <= 0n) return 0;
  // Keep the bar visual-only. Exact values remain the source of truth in the
  // legend and the details disclosure; two decimal places are enough here.
  return Math.min(100, Number((value * 10_000n) / total) / 100);
}

export function BillingFundingChart({ rows }: { rows: BillingUsageRow[] }) {
  const estimated = creditsLabel(rows, "estimated_credits_micros");
  const values = lanes.map((lane) => ({
    ...lane,
    display: creditsLabel(rows, lane.key),
    known: knownTotal(rows, lane.key),
  }));
  const totalKnown = values.reduce((sum, lane) => sum + lane.known.sum, 0n);
  const incomplete = Math.max(
    estimated.unknown,
    ...values.map((lane) => lane.display.unknown),
  );
  const hasKnownFunding = totalKnown > 0n;
  const barLabel = hasKnownFunding
    ? values
        .filter((lane) => lane.known.sum > 0n)
        .map(
          (lane) =>
            `${lane.label}: ${lane.display.text} credits (${percentage(lane.known.sum, totalKnown)}%)`,
        )
        .join(", ")
    : "No known funding values for this period";

  return (
    <section className="funding-visual" aria-label="Funding composition">
      <header>
        <div>
          <h4>Funding composition</h4>
          <p className="fine-print">How the estimated period cost is covered</p>
        </div>
        <strong className="funding-visual-total">
          {estimated.text} <small>credits</small>
        </strong>
      </header>
      <div
        className={`funding-track${hasKnownFunding ? "" : " funding-track-empty"}`}
        role="img"
        aria-label={`${incomplete > 0 ? "Known funding" : "Funding"}: ${barLabel}`}
      >
        {values.map((lane) => (
          <span
            aria-hidden="true"
            className="funding-segment"
            key={lane.key}
            style={{
              backgroundColor: lane.color,
              width: `${percentage(lane.known.sum, totalKnown)}%`,
            }}
          />
        ))}
      </div>
      <div className="funding-visual-legend">
        {values.map((lane) => (
          <div className="funding-legend-item" key={lane.key}>
            <span
              aria-hidden="true"
              className="funding-legend-swatch"
              style={{ backgroundColor: lane.color }}
            />
            <span>
              <span className="funding-legend-label">{lane.label}</span>
              <strong>{lane.display.text} credits</strong>
            </span>
          </div>
        ))}
      </div>
      {incomplete > 0 && (
        <p className="fine-print funding-visual-note">
          Some funding values are unavailable. The bar shows only known funding
          and the displayed estimated cost remains a lower bound where marked ≥.
        </p>
      )}
    </section>
  );
}

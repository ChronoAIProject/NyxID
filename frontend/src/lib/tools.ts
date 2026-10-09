import type { ToolOffering } from "@/schemas/tools";
import { parseCredits } from "@/lib/credits";

/** Effective platform price of one operation: its synced price, else the base. */
export function operationPrice(tool: ToolOffering, operation: string) {
  const price = tool.pricing.platform;
  if (price === "free") return null;
  return (
    price.operations?.find(
      (candidate) =>
        candidate.operation === operation &&
        (!candidate.sync_status || candidate.sync_status === "synced"),
    )?.credits_per_unit ?? price.credits_per_unit
  );
}

export function toolPrice(tool: ToolOffering) {
  const price = tool.pricing.platform;
  if (price === "free") return "Free";
  if (price.operations?.length && tool.operations.length) {
    const prices = tool.operations.map(
      (operation) => operationPrice(tool, operation.name)!,
    );
    const min = prices.reduce((a, b) =>
      parseCredits(a) < parseCredits(b) ? a : b,
    );
    const max = prices.reduce((a, b) =>
      parseCredits(a) > parseCredits(b) ? a : b,
    );
    if (min !== max) return `From ${min} to ${max} credits per request`;
    return `${min} credits / request`;
  }
  return `${price.credits_per_unit} credits / ${price.metric === "requests" ? "request" : price.metric}`;
}

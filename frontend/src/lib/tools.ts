import type { ToolOffering } from "@/schemas/tools";
export function toolPrice(tool: ToolOffering) {
  const price = tool.pricing.platform;
  return price === "free"
    ? "Free"
    : `${price.credits_per_unit} credits / ${price.metric === "requests" ? "request" : price.metric}`;
}

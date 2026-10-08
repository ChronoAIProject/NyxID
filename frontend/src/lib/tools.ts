import type { ToolOffering } from "@/schemas/tools";
export function toolPrice(tool: ToolOffering) {
  const price = tool.pricing.platform;
  return price === "free" || /^0+(\.0+)?$/.test(price.credits_per_unit)
    ? "Free"
    : `${price.credits_per_unit} credits / ${price.metric === "requests" ? "request" : price.metric}`;
}

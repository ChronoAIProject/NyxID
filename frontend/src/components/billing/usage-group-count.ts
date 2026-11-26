import { translateUi } from "@/i18n/use-ui-translation";
import type { Dimension } from "@/lib/billing-usage";

export function usageGroupCount(dimension: Dimension, count: number) {
  switch (dimension) {
    case "service":
      return translateUi("{{count}} service", { count });
    case "operation":
      return translateUi("{{count}} operation", { count });
    case "model":
      return translateUi("{{count}} model", { count });
    case "agent":
      return translateUi("{{count}} agent", { count });
    case "layer":
      return translateUi("{{count}} billing layer", { count });
    case "metric":
      return translateUi("{{count}} metric", { count });
  }
}

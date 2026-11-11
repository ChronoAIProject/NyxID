import { useUiTranslation } from "@/i18n/use-ui-translation";
import {
  BillingMultiSelect,
  type BillingMultiSelectProps,
} from "./billing-multi-select";

type BillingMetricPickerProps = Pick<
  BillingMultiSelectProps,
  "values" | "onChange" | "options" | "children" | "open" | "onOpenChange"
>;

export function BillingMetricPicker(props: BillingMetricPickerProps) {
  const { t } = useUiTranslation();
  return (
    <BillingMultiSelect
      {...props}
      label={t("Metrics")}
      itemLabel={t("Metric")}
      capitalize
    />
  );
}

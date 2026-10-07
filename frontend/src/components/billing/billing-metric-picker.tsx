import {
  BillingMultiSelect,
  type BillingMultiSelectProps,
} from "./billing-multi-select";

type BillingMetricPickerProps = Pick<
  BillingMultiSelectProps,
  "values" | "onChange" | "options" | "children" | "open" | "onOpenChange"
>;

export function BillingMetricPicker(props: BillingMetricPickerProps) {
  return (
    <BillingMultiSelect
      {...props}
      label="Metrics"
      itemLabel="Metric"
      capitalize
    />
  );
}

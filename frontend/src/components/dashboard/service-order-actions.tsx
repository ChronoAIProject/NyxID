import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Button } from "@/components/ui/button";
import type { ServiceGroupOrder } from "@/hooks/use-service-group-order";

export function ServiceOrderActions({
  order,
  formId,
}: {
  readonly order: ServiceGroupOrder;
  readonly formId?: string;
}) {
  useUiTranslation();
  return (
    <>
      <span className="type-row-title">{translateUi("Discovery order")}</span>
      <Button
        type="button"
        variant="outline"
        disabled={order.busy}
        onClick={order.cancel}
      >{translateUi("Cancel")}</Button>
      <Button
        type="submit"
        form={formId}
        variant="primary"
        isLoading={order.busy}
        disabled={
          order.busy ||
          order.blocked ||
          !order.dirty ||
          order.failure === "stale" ||
          order.failure === "capacity"
        }
      >{translateUi("Save")}</Button>
    </>
  );
}

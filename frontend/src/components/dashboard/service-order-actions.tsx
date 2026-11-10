import { useUiTranslation } from "@/i18n/use-ui-translation";
import { Button } from "@/components/ui/button";
import type { ServiceGroupOrder } from "@/hooks/use-service-group-order";

export function ServiceOrderActions({
  order,
  formId,
}: {
  readonly order: ServiceGroupOrder;
  readonly formId?: string;
}) {
  const { t } = useUiTranslation();
  return (
    <>
      <span className="text-12 font-medium">{t("Discovery order")}</span>
      <Button
        type="button"
        variant="outline"
        disabled={order.busy}
        onClick={order.cancel}
      >{t("Cancel")}</Button>
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
      >{t("Save")}</Button>
    </>
  );
}

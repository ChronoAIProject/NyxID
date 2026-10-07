import { Button } from "@/components/ui/button";
import type { ServiceGroupOrder } from "@/hooks/use-service-group-order";

export function ServiceOrderActions({
  order,
  formId,
}: {
  readonly order: ServiceGroupOrder;
  readonly formId?: string;
}) {
  return (
    <>
      <span className="text-12 font-medium">Agent order</span>
      <Button
        type="button"
        variant="outline"
        disabled={order.busy}
        onClick={order.cancel}
      >
        Cancel
      </Button>
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
      >
        Save
      </Button>
    </>
  );
}

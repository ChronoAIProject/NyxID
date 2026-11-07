import { useAdminBillingService } from "@/hooks/use-admin-billing";
import { publicModelSchema, type AdminModel } from "@/schemas/platform-models";
import { SaveError } from "./shared";

/** Preview the allowlisted customer DTO; never serialize the admin model itself. */
export function PublicPreview({ model }: { readonly model: AdminModel }) {
  const billing = useAdminBillingService(model.billing_service_id ?? "");
  const preview = publicModelSchema.parse({
    id: model.public_id,
    display_name: model.display_name,
    description: model.description,
    capabilities: model.capabilities,
    availability:
      model.execution_ready && model.enabled && model.lifecycle === "active"
        ? "available"
        : "unavailable",
    retail_prices:
      billing.data?.effective.model_prices
        ?.filter((p) => p.model_id === model.id && p.lane === "platform_key")
        .map((p) => ({
          variant: p.variant,
          available: p.available,
          components: p.available ? p.components : [],
        })) ?? [],
  });
  return (
    <section className="space-y-3">
      <p className="type-secondary text-muted-foreground">
        Public metadata and retail prices only. Each person still needs
        entitlement to see this model.
      </p>
      <SaveError error={billing.error} />
      {billing.isFetching ? (
        <p className="type-body">Loading prices…</p>
      ) : (
        <pre className="overflow-auto rounded-xl border border-border/50 p-4 type-code">
          {JSON.stringify(preview, null, 2)}
        </pre>
      )}
    </section>
  );
}

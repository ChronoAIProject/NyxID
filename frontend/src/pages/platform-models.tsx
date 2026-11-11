import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { usePlatformModels } from "@/hooks/use-platform-models";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { publicModelSchema } from "@/schemas/platform-models";
import { PageHeader } from "@/components/shared/page-header";
import { Button } from "@/components/ui/button";
import { PriceRows, SaveError } from "@/components/platform-models/shared";

export function PlatformModelsPage() {
  useUiTranslation();
  const enabled = useFeature(FEATURE_FLAG.INFERENCE_PLATFORM_MODELS);
  const query = usePlatformModels();
  if (!enabled) return null;
  const models =
    query.data?.pages.flatMap((page) =>
      page.models.map((model) => publicModelSchema.parse(model)),
    ) ?? [];
  return (
    <div className="space-y-6">
      <PageHeader
        title={translateUi("Platform Models")}
        description={translateUi("Models available to your account. Entitlement, execution readiness and prices are checked independently.")}
      />
      <SaveError error={query.error} />
      {query.isLoading && <p className="type-body">{translateUi("Loading models…")}</p>}
      {!query.isLoading && !query.error && models.length === 0 && (
        <p className="type-body text-muted-foreground">{translateUi("No platform models are available to your account.")}</p>
      )}
      <div className="grid gap-4 lg:grid-cols-2">
        {models.map((model) => (
          <article
            key={model.id}
            className="overflow-hidden rounded-xl border border-border/50"
          >
            <div className="space-y-2 p-5">
              <h2 className="type-card-title">{model.display_name}</h2>
              <p className="type-code break-all text-muted-foreground">
                {model.id}
              </p>
              <p className="type-body">{model.description}</p>
              <p className="type-secondary text-muted-foreground">
                {model.availability === "available"
                  ? translateUi("Ready for use")
                  : model.availability === "request_dependent"
                    ? translateUi("Availability checked per request")
                    : translateUi("Execution unavailable")}
              </p>
              <p className="type-secondary text-muted-foreground">
                {model.capabilities.protocols.join(" · ")} ·{" "}
                {model.capabilities.input_modalities.join(", ")} →{" "}
                {model.capabilities.output_modalities.join(", ")}
              </p>
            </div>
            {model.retail_prices.length === 0 && (
              <p className="px-5 pb-5 type-secondary text-muted-foreground">{translateUi("Retail prices unavailable")}</p>
            )}
            {model.retail_prices.map((price, i) => (
              <section key={i} className="border-t border-border/30">
                <h3 className="px-4 pt-3 type-field-label">
                  {Object.entries(price.variant)
                    .map(([k, v]) => `${k}: ${v}`)
                    .join(" · ") || translateUi("Retail prices")}
                </h3>
                {price.available ? (
                  <PriceRows components={price.components} />
                ) : (
                  <p className="p-4 type-secondary text-muted-foreground">{translateUi("Retail prices unavailable")}</p>
                )}
              </section>
            ))}
          </article>
        ))}
      </div>
      {query.hasNextPage && (
        <Button
          variant="outline"
          disabled={query.isFetchingNextPage}
          onClick={() => void query.fetchNextPage()}
        >{translateUi("Load more models")}</Button>
      )}
    </div>
  );
}

import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { ContractPicker } from "./contract-picker";
import { Checkbox } from "@/components/ui/checkbox";
import {
  type CertifiedContract,
  type UsageContract,
  attestation,
  validSource,
} from "@/schemas/model-contracts";
import { titleCase } from "@/schemas/platform-models";
import { useAdminBillingService } from "@/hooks/use-admin-billing";
import { useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useModelTariffs,
  useModelContracts,
  usePlatformModelChange,
  adminModelPath,
} from "@/hooks/use-platform-models";
import {
  type AdminModel,
  type Tariff,
  exactPrice,
} from "@/schemas/platform-models";
import { Form, useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { NativeSelect } from "@/components/ui/native-select";
import { EditErrors, Field, PriceRows, SaveError } from "./shared";

const objectJson = z.string().refine((s) => {
  try {
    const x: unknown = JSON.parse(s);
    return !!x && typeof x === "object" && !Array.isArray(x);
  } catch {
    return false;
  }
}, "Enter a JSON object");
const component = z
  .object({
    key: z.string().min(1),
    metric: z.string().min(1),
    selector: z.object({
      kind: z.string().min(1),
      ttl: z.string().optional(),
      modality: z.string().optional(),
    }),
    credits_per_unit: exactPrice,
    explicit_free: z.boolean(),
    max_quantity: z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER),
  })
  .refine(
    (c) => /^0(?:\.0+)?$/.test(c.credits_per_unit) === c.explicit_free,
    "Zero must be explicitly free; positive prices cannot be free",
  );
const schema = z
  .object({
    certified: z.boolean(),
    confirmed: z.boolean(),
    reference: z.string(),
    publicModel: z.string(),
    lane: z.enum(["platform_key", "byok"]),
    source: z.string(),
    variant: objectJson,
    contract: objectJson,
    bounds: objectJson,
    components: z.array(component).min(1),
  })
  .superRefine((v, ctx) => {
    if (v.certified && !v.confirmed)
      ctx.addIssue({
        code: "custom",
        path: ["confirmed"],
        message: "Confirm your verification before creating a revision",
      });
    if (v.certified && !v.reference.trim())
      ctx.addIssue({
        code: "custom",
        path: ["reference"],
        message: "Enter a verification reference",
      });
    if (
      !validSource(
        v.certified ? attestation(v.publicModel, v.reference) : v.source,
      )
    )
      ctx.addIssue({
        code: "custom",
        path: [v.certified ? "reference" : "source"],
        message:
          "Verification source must be at most 1000 UTF-8 bytes with no control characters",
      });
  });
type Values = z.infer<typeof schema>;
function defaults(
  model: AdminModel,
  definitions: CertifiedContract[],
  t?: Tariff,
): Values {
  const definition = definitions[0];
  return {
    certified: !!definition,
    confirmed: false,
    reference: "",
    publicModel: model.public_id,
    lane: t?.lane === "byok" ? "byok" : "platform_key",
    source: t?.source ?? "",
    variant: JSON.stringify(t?.variant ?? {}, null, 2),
    contract: definition
      ? JSON.stringify(
          t?.contract.version === definition.version &&
            t?.contract.fixture_sha256 === definition.fixture_sha256
            ? t.contract
            : definition.default_contract,
        )
      : t
        ? JSON.stringify(t.contract, null, 2)
        : "",
    bounds: t ? JSON.stringify(t.bounds, null, 2) : "",
    components:
      t?.components.map(
        ({
          key,
          metric,
          selector,
          credits_per_unit,
          explicit_free,
          max_quantity,
        }) => ({
          key,
          metric,
          selector,
          credits_per_unit,
          explicit_free,
          max_quantity,
        }),
      ) ?? [],
  };
}
function TariffForm({
  model,
  definitions,
  template,
  done,
  reload,
}: {
  readonly model: AdminModel;
  readonly definitions: CertifiedContract[];
  readonly template?: Tariff;
  readonly done: () => void;
  readonly reload: () => void;
}) {
  useUiTranslation();
  const form = useAppForm<Values>({
    resolver: zodResolver(schema),
    defaultValues: defaults(model, definitions, template),
    mode: "onChange",
  });
  const change = usePlatformModelChange();
  return (
    <Form {...form}>
      <form
        className="space-y-4 rounded-xl border border-border/50 p-4"
        onSubmit={form.handleSubmit(async (v) => {
          try {
            await change.mutateAsync({
              path: `${adminModelPath(model.id)}/tariffs`,
              method: "post",
              body: {
                lane: v.lane,
                source: v.certified
                  ? attestation(model.public_id, v.reference)
                  : v.source,
                revision: model.revision,
                components: v.components.map((c) => ({
                  ...c,
                  selector:
                    c.selector.kind === "cache_write"
                      ? { kind: c.selector.kind, ttl: c.selector.ttl }
                      : c.selector.kind === "cached_modality"
                        ? {
                            kind: c.selector.kind,
                            modality: c.selector.modality,
                          }
                        : { kind: c.selector.kind },
                })),
                variant: JSON.parse(v.variant),
                contract: JSON.parse(v.contract),
                bounds: JSON.parse(v.bounds),
              },
            });
            done();
          } catch {
            /* shown below */
          }
        })}
      >
        <h3 className="type-section-heading">{translateUi("New price revision")}</h3>
        <p className="type-secondary text-muted-foreground">{translateUi("Use a certified adapter usage contract, fixture digest and request bounds. Prices are exact credits per base unit. A revision stays pending until all rates sync; incomplete prices never fall back to a base price.")}</p>
        <fieldset disabled={change.isPending} className="space-y-4">
          <Field title={translateUi("Credential lane")}>
            <NativeSelect {...form.register("lane")}>
              <option value="platform_key">{translateUi("Platform key")}</option>
              <option value="byok">{translateUi("BYOK")}</option>
            </NativeSelect>
          </Field>
          {definitions.length > 0 ? (
            <>
              <ContractPicker
                definitions={definitions}
                value={JSON.parse(form.watch("contract")) as UsageContract}
                onChange={(value) =>
                  form.setValue("contract", JSON.stringify(value))
                }
              />
              <Field title={translateUi("Verification reference (link or note)")}>
                <Input {...form.register("reference")} />
              </Field>
              <Field title={translateUi("I verified this model's limits, features and prices")}>
                <Checkbox
                  checked={form.watch("confirmed")}
                  onCheckedChange={(checked) =>
                    form.setValue("confirmed", checked === true)
                  }
                />
              </Field>
            </>
          ) : (
            <Field title={translateUi("Verification source")}>
              <Input {...form.register("source")} />
            </Field>
          )}
          {(
            [
              "variant",
              ...(definitions.length ? [] : ["contract"]),
              "bounds",
            ] as ("variant" | "contract" | "bounds")[]
          ).map((field) => (
            <Field key={field} title={translateUi("{{value1}} JSON", { value1: translateUi(titleCase(field)) })}>
              <Textarea
                rows={field === "contract" ? 10 : 4}
                {...form.register(field)}
              />
            </Field>
          ))}
          {form.watch("components").map((c, i) => (
            <div
              key={i}
              className="grid gap-3 rounded-lg border border-border/30 p-3 sm:grid-cols-2"
            >
              <Field title={translateUi("Component {{value1}} key", { value1: i + 1 })}>
                <Input {...form.register(`components.${i}.key`)} />
              </Field>
              <Field title={translateUi("Billing metric")}>
                <Input
                  {...form.register(`components.${i}.metric`)}
                  placeholder="input_tokens"
                />
              </Field>
              <Field title={translateUi("Selector")}>
                <NativeSelect
                  {...form.register(`components.${i}.selector.kind`)}
                >
                  {[
                    "input",
                    "output",
                    "reasoning",
                    "cache_read",
                    "cache_write",
                    "image_input",
                    "image_output",
                    "audio_input",
                    "audio_output",
                    "cached_modality",
                    "images",
                    "requests",
                  ].map((kind) => (
                    <option key={kind} value={kind}>
                      {translateUi(titleCase(kind))}
                    </option>
                  ))}
                </NativeSelect>
              </Field>
              {c.selector.kind === "cache_write" && (
                <Field title={translateUi("Cache TTL")}>
                  <NativeSelect
                    value={c.selector.ttl ?? ""}
                    onChange={(e) =>
                      form.setValue(
                        `components.${i}.selector.ttl`,
                        e.target.value,
                      )
                    }
                  >
                    <option value="">{translateUi("Select TTL")}</option>
                    {[
                      "five_minutes",
                      "one_hour",
                      "thirty_minutes",
                      "twelve_hours",
                    ].map((ttl) => (
                      <option key={ttl} value={ttl}>
                        {translateUi(titleCase(ttl))}
                      </option>
                    ))}
                  </NativeSelect>
                </Field>
              )}
              {c.selector.kind === "cached_modality" && (
                <Field title={translateUi("Cached modality")}>
                  <NativeSelect
                    value={c.selector.modality ?? ""}
                    onChange={(e) =>
                      form.setValue(
                        `components.${i}.selector.modality`,
                        e.target.value,
                      )
                    }
                  >
                    <option value="">{translateUi("Select modality")}</option>
                    <option value="image">{translateUi("Image")}</option>
                    <option value="audio">{translateUi("Audio")}</option>
                  </NativeSelect>
                </Field>
              )}
              <Field title={translateUi("Credits per unit")}>
                <Input
                  {...form.register(`components.${i}.credits_per_unit`)}
                  inputMode="decimal"
                />
              </Field>
              <Field title={translateUi("Maximum quantity")}>
                <Input
                  type="number"
                  {...form.register(`components.${i}.max_quantity`, {
                    valueAsNumber: true,
                  })}
                />
              </Field>
              <Field title={translateUi("Explicitly free")}>
                <NativeSelect
                  value={c.explicit_free ? "yes" : "no"}
                  onChange={(e) =>
                    form.setValue(
                      `components.${i}.explicit_free`,
                      e.target.value === "yes",
                    )
                  }
                >
                  <option value="no">{translateUi("No")}</option>
                  <option value="yes">{translateUi("Yes (zero price)")}</option>
                </NativeSelect>
              </Field>
              <Button
                type="button"
                variant="ghost"
                onClick={() =>
                  form.setValue(
                    "components",
                    form.getValues("components").filter((_, n) => n !== i),
                  )
                }
              >{translateUi("Remove component ")}{i + 1}
              </Button>
            </div>
          ))}
          <Button
            type="button"
            variant="outline"
            onClick={() =>
              form.setValue("components", [
                ...form.getValues("components"),
                {
                  key: "",
                  metric: "input_tokens",
                  selector: { kind: "input" },
                  credits_per_unit: "",
                  explicit_free: false,
                  max_quantity: 0,
                },
              ])
            }
          >{translateUi("Add price component")}</Button>
        </fieldset>
        <SaveError error={change.error} reload={reload} />
        <EditErrors />
        <div className="flex justify-end gap-2">
          <Button type="button" variant="outline" onClick={done}>{translateUi("Cancel")}</Button>
          <Button
            variant="primary"
            disabled={!form.formState.isValid || change.isPending}
          >{translateUi("Create pending revision")}</Button>
        </div>
      </form>
    </Form>
  );
}
export function TariffsEditor({
  model,
  readOnly,
  reload,
}: {
  readonly model: AdminModel;
  readonly readOnly: boolean;
  readonly reload: () => void;
}) {
  useUiTranslation();
  const tariffs = useModelTariffs(model.id);
  const contracts = useModelContracts();
  const definitions =
    contracts.data?.filter((d) => d.family === model.family) ?? [];
  const billing = useAdminBillingService(model.billing_service_id ?? "");
  const change = usePlatformModelChange();
  const [editing, setEditing] = useState<{ template?: Tariff } | null>(null);
  const [retiring, setRetiring] = useState<string | null>(null);
  return (
    <section className="space-y-5">
      <p className="type-body text-muted-foreground">{translateUi("Immutable revisions, newest first (latest 100). Publishing synchronizes the logical billing service; a revision becomes active only after every rate is ready. Execution readiness is separate.")}</p>
      <SaveError
        error={
          tariffs.error ?? billing.error ?? contracts.error ?? change.error
        }
        reload={reload}
      />
      {billing.data?.effective.model_prices
        ?.filter((p) => p.model_id === model.id)
        .map((p, i) => (
          <div
            key={i}
            className="rounded-lg border border-border/40 p-4 space-y-2"
          >
            <h3 className="type-card-title">
              {translateUi(titleCase(p.lane))}{translateUi(" price readiness")}</h3>
            <p className="type-body">
              {p.available
                ? translateUi("Prices ready")
                : translateUi("Prices unavailable: {{value1}}", { value1: translateUi(titleCase(p.blocking_reason ?? "unknown")) })}
            </p>
            <p className="type-secondary text-muted-foreground">
              {Object.entries(p.variant)
                .map(([k, v]) => `${k}: ${v}`)
                .join(" · ") || translateUi("Default variant")}
            </p>
            <p className="type-secondary">{translateUi("Active revision:")}{" "}
              {tariffs.data?.find((t) => t.id === p.active_tariff_id)
                ?.revision ?? translateUi("None")}{" "}{translateUi("· Pending revision:")}{" "}
              {tariffs.data?.find((t) => t.id === p.pending_tariff_id)
                ?.revision ?? translateUi("None")}
            </p>
          </div>
        ))}
      {!readOnly && !editing && (
        <Button
          variant="outline"
          disabled={
            !model.billing_service_id ||
            (model.family !== "open_model" && definitions.length === 0)
          }
          onClick={() => setEditing({})}
        >{translateUi("New price revision")}</Button>
      )}
      {editing &&
        !readOnly &&
        (model.family === "open_model" || definitions.length > 0) && (
          <TariffForm
            key={`${model.id}:${editing.template?.id ?? "new"}`}
            definitions={definitions}
            model={model}
            template={editing.template}
            done={() => setEditing(null)}
            reload={reload}
          />
        )}
      {!model.billing_service_id && (
        <p className="type-secondary text-muted-foreground">{translateUi("Set the logical billing service in Public contract before adding prices.")}</p>
      )}
      {model.billing_service_id && !readOnly && (
        <Button
          variant="outline"
          disabled={change.isPending}
          onClick={() => {
            void change
              .mutateAsync({
                path: `/admin/billing/services/${encodeURIComponent(model.billing_service_id!)}/resync`,
                method: "post",
                body: {},
              })
              .catch(() => {});
          }}
        >{translateUi("Publish / retry rate sync")}</Button>
      )}
      {tariffs.isLoading && (
        <p className="type-body">{translateUi("Loading price revisions…")}</p>
      )}
      {tariffs.data?.length === 0 && (
        <p className="type-body text-muted-foreground">{translateUi("No price revisions. This model cannot be priced for execution.")}</p>
      )}
      {tariffs.data?.map((tariff) => (
        <article
          key={tariff.id}
          className="overflow-hidden rounded-xl border border-border/50"
        >
          <div className="space-y-2 p-4">
            <h3 className="type-card-title">{translateUi("Revision ")}{tariff.revision} · {translateUi(titleCase(tariff.status))} ·{" "}
              {translateUi(titleCase(tariff.lane))}
            </h3>
            <p className="type-secondary text-muted-foreground">
              {Object.entries(tariff.variant)
                .map(([k, v]) => `${k}: ${v}`)
                .join(" · ") || translateUi("Default variant")}
            </p>
            {tariff.sync_error && (
              <p className="type-secondary text-destructive">{translateUi("Sync: ")}{tariff.sync_error}
              </p>
            )}
          </div>
          <PriceRows components={tariff.components} />
          <details className="p-4">
            <summary className="type-body cursor-pointer">{translateUi("Verification and bounds (private)")}</summary>
            <pre className="mt-3 overflow-auto type-code">
              {JSON.stringify(
                {
                  source: tariff.source,
                  contract: tariff.contract,
                  bounds: tariff.bounds,
                },
                null,
                2,
              )}
            </pre>
          </details>
          {!readOnly && (
            <div className="flex flex-wrap gap-2 p-4">
              <Button
                variant="outline"
                onClick={() => setEditing({ template: tariff })}
              >{translateUi("Use as template")}</Button>
              {tariff.status !== "retired" &&
                (retiring === tariff.id ? (
                  <>
                    <span className="type-secondary">{translateUi("Retiring removes this rate from new use.")}</span>
                    <Button variant="outline" onClick={() => setRetiring(null)}>{translateUi("Cancel")}</Button>
                    <Button
                      variant="destructive"
                      disabled={change.isPending}
                      onClick={() => {
                        void change
                          .mutateAsync({
                            path: `${adminModelPath(model.id)}/tariffs/${encodeURIComponent(tariff.id)}/retire`,
                            method: "post",
                            body: { revision: model.revision },
                          })
                          .then(() => setRetiring(null))
                          .catch(() => {});
                      }}
                    >{translateUi("Confirm retire revision")}</Button>
                  </>
                ) : (
                  <Button
                    variant="outline"
                    onClick={() => setRetiring(tariff.id)}
                  >{translateUi("Retire revision")}</Button>
                ))}
            </div>
          )}
        </article>
      ))}
    </section>
  );
}

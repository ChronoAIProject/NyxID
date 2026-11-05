import { useEffect, useState } from "react";
import { useAddCrmField, useUpdateCrmField } from "@/hooks/use-crm";
import { Button } from "@/components/ui/button";
import { Form, useAppForm } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { CrmObject, CrmField } from "@/schemas/crm";

type FieldDraft = {
  expectedRevision: number;
  label: string;
  required: boolean;
  archived: boolean;
  options: Extract<CrmField, { type: "select" }>["options"];
};
function defaults(field: CrmField, expectedRevision: number): FieldDraft {
  return {
    expectedRevision,
    label: field.label,
    required: field.required,
    archived: field.archived,
    options: field.type === "select" ? field.options : [],
  };
}
function FieldSettings({
  object,
  field,
  ownerId,
}: {
  readonly object: CrmObject;
  readonly field: CrmField;
  readonly ownerId?: string;
}) {
  const [retry, setRetry] = useState<{ payload: string; key: string } | null>(
    null,
  );
  const form = useAppForm<FieldDraft>({
    defaultValues: defaults(field, object.schema_revision),
  });
  const [error, setError] = useState<string>();
  const patch = useUpdateCrmField(ownerId);
  const stage = object.pipeline?.stage_field === field.key;
  const dirty = form.formState.isDirty;
  const baseline = form.watch("expectedRevision");
  const outdated = baseline < object.schema_revision;
  useEffect(() => {
    if (!dirty && !patch.isPending && baseline < object.schema_revision) {
      form.reset(defaults(field, object.schema_revision));
    }
  }, [baseline, dirty, field, form, object.schema_revision, patch.isPending]);
  const options = form.watch("options");
  function reload() {
    form.reset(defaults(field, object.schema_revision));
    setError(undefined);
  }
  return (
    <Form {...form}>
      <form
        className="space-y-2 rounded-lg border border-border p-3"
        onSubmit={form.handleSubmit(async (draft) => {
          setError(undefined);
          try {
            const payload = JSON.stringify({ revision: baseline, draft });
            const attempt =
              retry?.payload === payload
                ? retry
                : { payload, key: crypto.randomUUID() };
            setRetry(attempt);
            const saved = await patch.mutateAsync({
              idempotencyKey: attempt.key,
              objectKey: object.key,
              fieldKey: field.key,
              expectedSchemaRevision: baseline,
              change: {
                label: draft.label,
                required: draft.required,
                archived: draft.archived,
                ...(field.type === "select" ? { options: draft.options } : {}),
              },
            });
            setRetry(null);
            const savedField = saved.fields.find(
              (item) => item.key === field.key,
            );
            form.reset(
              savedField
                ? defaults(savedField, saved.schema_revision)
                : { ...draft, expectedRevision: saved.schema_revision },
            );
          } catch (cause) {
            setError(
              cause instanceof Error ? cause.message : "Could not save field.",
            );
          }
        })}
      >
        <fieldset disabled={patch.isPending} className="space-y-2">
          <p className="text-11 text-text-tertiary">
            {field.key} · {field.type}
          </p>
          <Label>
            Field label
            <Input
              aria-label={`${field.key} label`}
              maxLength={200}
              {...form.register("label", {
                required: true,
                maxLength: 200,
                validate: (value) => Boolean(value.trim()),
              })}
            />
          </Label>
          <label className="flex gap-2 text-12">
            <input
              type="checkbox"
              disabled={stage}
              {...form.register("required")}
            />
            Required{stage ? " (pipeline stage)" : ""}
          </label>
          <label className="flex gap-2 text-12">
            <input
              type="checkbox"
              disabled={stage}
              {...form.register("archived")}
            />
            Archived (stored values stay readable)
          </label>
          {field.type === "select" ? (
            <div className="space-y-2">
              {options.map((option, index) => (
                <div
                  key={option.value}
                  className="flex flex-wrap items-center gap-2"
                >
                  <span className="text-11">{option.value}</span>
                  <Input
                    className="w-40"
                    aria-label={`${option.value} option label`}
                    maxLength={200}
                    {...form.register(`options.${index}.label`, {
                      required: true,
                      maxLength: 200,
                    })}
                  />
                  <label className="flex gap-2 text-11">
                    <input
                      type="checkbox"
                      {...form.register(`options.${index}.archived`)}
                    />
                    Archived
                  </label>
                </div>
              ))}
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={options.length >= (stage ? 50 : 100)}
                onClick={() =>
                  form.setValue("options", [
                    ...options,
                    {
                      value: `option_${crypto.randomUUID().slice(0, 8)}`,
                      label: "New option",
                      archived: false,
                      color: null,
                    },
                  ])
                }
              >
                Add option
              </Button>
            </div>
          ) : null}
          <Button
            size="sm"
            disabled={!dirty || outdated}
            isLoading={patch.isPending}
          >
            Save field
          </Button>
          {outdated && dirty ? (
            <p role="alert" className="text-12">
              Schema changed while you were editing. Your draft is retained.{" "}
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={reload}
              >
                Reload {field.key}
              </Button>
            </p>
          ) : null}
          {error ? (
            <p role="alert" className="text-12 text-destructive">
              {error}{" "}
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={reload}
              >
                Reload {field.key}
              </Button>
            </p>
          ) : null}
        </fieldset>
      </form>
    </Form>
  );
}

type NewFieldDraft = {
  key: string;
  label: string;
  kind: CrmField["type"];
  currency: string;
  target: string;
  format: "plain" | "percent";
  cardinality: "one" | "many";
};
const NEW_FIELD: NewFieldDraft = {
  key: "",
  label: "",
  kind: "text",
  currency: "USD",
  target: "company",
  format: "plain",
  cardinality: "one",
};
export function CrmSchemaEditor({
  object,
  ownerId,
  objects = [object],
}: {
  readonly object: CrmObject;
  readonly ownerId?: string;
  readonly objects?: readonly CrmObject[];
}) {
  const add = useAddCrmField(ownerId);
  const [retry, setRetry] = useState<{ payload: string; key: string } | null>(
    null,
  );
  const form = useAppForm<NewFieldDraft>({ defaultValues: NEW_FIELD });
  const [error, setError] = useState<string>();
  const kind = form.watch("kind");
  return (
    <section aria-label="Object schema" className="space-y-3">
      <p className="text-12 text-text-secondary">
        Schema revision {object.schema_revision}. Keys and types are immutable.
        Up to 100 fields and 100 options per field, including archived
        definitions. Pipeline stages are required and support at most 50
        options.
      </p>
      <div className="grid gap-3 md:grid-cols-2">
        {object.fields.map((field) => (
          <FieldSettings
            key={`${object.owner_id}:${object.key}:${field.key}`}
            object={object}
            field={field}
            ownerId={ownerId}
          />
        ))}
      </div>
      <Form {...form}>
        <form
          className="space-y-3 rounded-lg border border-border p-4"
          onSubmit={form.handleSubmit(async (draft) => {
            setError(undefined);
            try {
              const type =
                draft.kind === "text"
                  ? { type: draft.kind, max_length: 1000 }
                  : draft.kind === "number"
                    ? { type: draft.kind, format: draft.format }
                    : draft.kind === "currency"
                      ? { type: draft.kind, currency: draft.currency }
                      : draft.kind === "select"
                        ? { type: draft.kind, options: [] }
                        : draft.kind === "record"
                          ? {
                              type: draft.kind,
                              target: draft.target,
                              cardinality: draft.cardinality,
                            }
                          : { type: draft.kind };
              const payload = JSON.stringify({
                revision: object.schema_revision,
                draft,
              });
              const attempt =
                retry?.payload === payload
                  ? retry
                  : { payload, key: crypto.randomUUID() };
              setRetry(attempt);
              await add.mutateAsync({
                idempotencyKey: attempt.key,
                objectKey: object.key,
                expectedSchemaRevision: object.schema_revision,
                field: {
                  key: draft.key,
                  label: draft.label,
                  required: false,
                  archived: false,
                  ...type,
                },
              });
              setRetry(null);
              form.reset(NEW_FIELD);
            } catch (cause) {
              setError(
                cause instanceof Error ? cause.message : "Could not add field.",
              );
            }
          })}
        >
          <fieldset disabled={add.isPending} className="space-y-3">
            <h3 className="text-13 font-semibold">Add custom field</h3>
            <Label>
              Stable field key
              <Input
                pattern="[a-z][a-z0-9_]*"
                maxLength={48}
                {...form.register("key", {
                  required: true,
                  maxLength: 48,
                  pattern: /^[a-z][a-z0-9_]{0,47}$/,
                })}
              />
            </Label>
            <Label>
              Label
              <Input
                maxLength={200}
                {...form.register("label", {
                  required: true,
                  maxLength: 200,
                  validate: (value) => Boolean(value.trim()),
                })}
              />
            </Label>
            <Label>
              Field type
              <select
                className="ml-2 rounded border border-border bg-background p-2"
                {...form.register("kind")}
              >
                {[
                  "text",
                  "number",
                  "currency",
                  "boolean",
                  "date",
                  "datetime",
                  "select",
                  "record",
                  "user",
                ].map((type) => (
                  <option key={type}>{type}</option>
                ))}
              </select>
            </Label>
            {kind === "number" ? (
              <Label>
                Number format
                <select {...form.register("format")}>
                  <option value="plain">Plain number</option>
                  <option value="percent">Percentage</option>
                </select>
              </Label>
            ) : null}
            {kind === "currency" ? (
              <Label>
                Currency
                <select {...form.register("currency")}>
                  {["USD", "EUR", "GBP", "SGD", "AUD", "CAD", "HKD", "CNY"].map(
                    (code) => (
                      <option key={code}>{code}</option>
                    ),
                  )}
                </select>
              </Label>
            ) : null}
            {kind === "record" ? (
              <>
                <Label>
                  Target object
                  <select
                    className="ml-2 rounded border border-border bg-background p-2"
                    {...form.register("target")}
                  >
                    {objects.map((item) => (
                      <option key={item.key} value={item.key}>
                        {item.plural_label}
                      </option>
                    ))}
                  </select>
                </Label>
                <Label>
                  Relationship
                  <select {...form.register("cardinality")}>
                    <option value="one">One record</option>
                    <option value="many">Many records</option>
                  </select>
                </Label>
              </>
            ) : null}
            <Button
              type="submit"
              isLoading={add.isPending}
              disabled={object.fields.length >= 100}
            >
              Add field
            </Button>
          </fieldset>
          {error ? (
            <p role="alert" className="text-12 text-destructive">
              {error}
            </p>
          ) : null}
        </form>
      </Form>
    </section>
  );
}

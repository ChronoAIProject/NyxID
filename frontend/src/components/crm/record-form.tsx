import { CrmOwnerContext } from "@/lib/crm/owner-context";
import { useId, useRef } from "react";
import { CrmFieldEditor } from "@/components/crm/field-editor";
import { Button } from "@/components/ui/button";
import { Form, useAppForm } from "@/components/ui/form";
import { useCreateCrmRecord } from "@/hooks/use-crm";
import { ApiError } from "@/lib/api-client";
import { formatCrmFieldInput, parseCrmFieldInput } from "@/lib/crm/field-kinds";
import {
  crmValidationDetailsSchema,
  type CrmField,
  type CrmObject,
  type CrmRecord,
} from "@/schemas/crm";

interface CrmRecordFormValues {
  readonly values: Record<string, unknown>;
}

export interface CrmRecordFormProps {
  readonly ownerId?: string;
  readonly object: CrmObject;
  readonly fields?: readonly string[];
  readonly title: string;
  readonly submitLabel?: string;
  readonly onCreated?: (record: CrmRecord) => void;
}

function includedFields(
  object: CrmObject,
  requested?: readonly string[],
): CrmField[] {
  const selected = requested ? new Set(requested) : null;
  return object.fields.filter(
    (field) =>
      !field.archived &&
      (field.required || selected === null || selected.has(field.key)),
  );
}

function defaults(fields: readonly CrmField[]) {
  return Object.fromEntries(
    fields.map((field) => [field.key, formatCrmFieldInput(field, undefined)]),
  );
}

function serverErrors(error: unknown): Readonly<Record<string, string>> {
  if (!(error instanceof ApiError) || error.errorCode !== 13401) return {};
  const parsed = crmValidationDetailsSchema.safeParse(
    error.errorResponse.errors !== undefined
      ? { errors: error.errorResponse.errors }
      : error.errorResponse.details,
  );
  if (!parsed.success) return {};
  return Object.fromEntries(
    parsed.data.errors.map((issue) => {
      const key = issue.path
        .replace(/^\$?\.?values\.?/, "")
        .replace(/^\//, "")
        .split(/[./]/)[0];
      return [key || "root", issue.message];
    }),
  );
}

export function CrmRecordForm({
  ownerId,
  object,
  fields: requestedFields,
  title,
  submitLabel = "Create record",
  onCreated,
}: CrmRecordFormProps) {
  const fields = includedFields(object, requestedFields);
  const headingId = useId();
  const create = useCreateCrmRecord(ownerId);
  const actionKey = useRef<string | null>(null);
  const form = useAppForm<CrmRecordFormValues>({
    defaultValues: { values: defaults(fields) },
  });
  const values = form.watch("values");
  const readOnly = "access" in object && object.access === "read";

  function setErrors(errors: Readonly<Record<string, string>>) {
    for (const [key, message] of Object.entries(errors)) {
      if (key === "root") form.setError("root", { message });
      else form.setError(`values.${key}`, { message });
    }
  }

  async function submit() {
    form.clearErrors();
    const parsed: Record<string, unknown> = {};
    const errors: Record<string, string> = {};
    for (const field of fields) {
      try {
        const value = parseCrmFieldInput(
          field,
          form.getValues(`values.${field.key}`),
        );
        const missing =
          value === null ||
          value === "" ||
          (Array.isArray(value) && value.length === 0);
        if (
          field.required &&
          (missing || (typeof value === "string" && !value.trim()))
        )
          errors[field.key] = `${field.label} is required.`;
        else parsed[field.key] = value;
      } catch (error) {
        errors[field.key] =
          error instanceof Error
            ? error.message
            : `Enter a valid ${field.label.toLocaleLowerCase()}.`;
      }
    }
    if (Object.keys(errors).length) {
      setErrors(errors);
      return;
    }
    actionKey.current ??= crypto.randomUUID();
    try {
      const record = await create.mutateAsync({
        object: object.key,
        values: parsed,
        idempotencyKey: actionKey.current,
      });
      actionKey.current = null;
      form.reset({ values: defaults(fields) });
      onCreated?.(record);
    } catch (error) {
      const errors = serverErrors(error);
      if (Object.keys(errors).length) setErrors(errors);
      else {
        form.setError("root", {
          message:
            error instanceof Error
              ? error.message
              : "Could not create the record.",
        });
      }
    }
  }

  return (
    <CrmOwnerContext value={ownerId ?? object.owner_id}>
      <section className="space-y-4" aria-labelledby={headingId}>
        <h3 id={headingId} className="text-15 font-semibold">
          {title}
        </h3>
        {readOnly ? (
          <p className="rounded-lg border border-dashed border-border p-3 text-12 text-text-tertiary">
            You have read-only access to this CRM.
          </p>
        ) : (
          <Form {...form}>
            <form className="space-y-4" onSubmit={form.handleSubmit(submit)}>
              {fields.map((field) => {
                const fieldError = form.formState.errors.values?.[field.key];
                const message =
                  fieldError &&
                  "message" in fieldError &&
                  typeof fieldError.message === "string"
                    ? fieldError.message
                    : undefined;
                return (
                  <CrmFieldEditor
                    key={field.key}
                    disabled={create.isPending}
                    field={field}
                    value={values[field.key]}
                    ownerId={ownerId}
                    error={message}
                    onChange={(value) => {
                      actionKey.current = null;
                      form.setValue(`values.${field.key}`, value);
                      form.clearErrors(`values.${field.key}`);
                    }}
                  />
                );
              })}
              {form.formState.errors.root?.message ? (
                <p role="alert" className="text-11 text-destructive">
                  {form.formState.errors.root.message}
                </p>
              ) : null}
              <Button
                type="submit"
                variant="primary"
                disabled={!form.formState.isDirty}
                isLoading={create.isPending}
              >
                {submitLabel}
              </Button>
            </form>
          </Form>
        )}
      </section>
    </CrmOwnerContext>
  );
}

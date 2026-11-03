import { useRef, useState } from "react";
import { Archive, ArchiveRestore, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { CrmActivityTimeline } from "@/components/crm/activity-timeline";
import { CrmFieldEditor } from "@/components/crm/field-editor";
import { CrmRecordForm } from "@/components/crm/record-form";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Form, useAppForm } from "@/components/ui/form";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  useCrmRecord,
  useCrmObject,
  useCrmRelated,
  useSetCrmRecordArchived,
  useUpdateCrmRecord,
} from "@/hooks/use-crm";
import { crmConflictRecord, crmValidationErrors } from "@/lib/crm/errors";
import {
  formatCrmFieldInput,
  parseCrmFieldInput,
  renderCrmCell,
} from "@/lib/crm/field-kinds";
import type {
  CrmField,
  CrmObject,
  CrmRecord,
  CrmRelatedGroup,
} from "@/schemas/crm";

interface RecordFormValues {
  readonly values: Record<string, unknown>;
  readonly protected: Record<string, boolean>;
}

interface RecordPatchDelta {
  readonly values: Record<string, unknown>;
  readonly protect: string[];
  readonly unprotect: string[];
}

function editorValues(object: CrmObject, record?: CrmRecord): RecordFormValues {
  const values: Record<string, unknown> = {};
  const protectedFields: Record<string, boolean> = {};
  for (const field of object.fields.filter(
    (candidate) => !candidate.archived,
  )) {
    values[field.key] =
      field.type === "record"
        ? field.cardinality === "many"
          ? (record?.links[field.key] ?? []).map((link) => link.id)
          : (record?.links[field.key]?.[0]?.id ?? null)
        : formatCrmFieldInput(field, record?.values[field.key]);
    protectedFields[field.key] = record?.overrides.includes(field.key) ?? false;
  }
  return { values, protected: protectedFields };
}

function parseValues(
  fields: readonly CrmField[],
  values: Readonly<Record<string, unknown>>,
): { data: Record<string, unknown>; errors: Record<string, string> } {
  const data: Record<string, unknown> = {};
  const errors: Record<string, string> = {};
  for (const field of fields) {
    try {
      const parsed = parseCrmFieldInput(field, values[field.key]);
      const blank =
        parsed === null ||
        parsed === "" ||
        (Array.isArray(parsed) && parsed.length === 0);
      if (field.required && blank) {
        errors[field.key] = `${field.label} is required.`;
      } else {
        data[field.key] = parsed;
      }
    } catch (error) {
      errors[field.key] =
        error instanceof Error
          ? error.message
          : `Enter a valid ${field.label.toLocaleLowerCase()}.`;
    }
  }
  return { data, errors };
}

function fieldError(
  errors: ReturnType<
    typeof useAppForm<RecordFormValues>
  >["formState"]["errors"],
  key: string,
) {
  const error = errors.values?.[key];
  return error && "message" in error && typeof error.message === "string"
    ? error.message
    : undefined;
}

function setFieldErrors(
  form: ReturnType<typeof useAppForm<RecordFormValues>>,
  errors: Readonly<Record<string, string>>,
) {
  for (const [key, message] of Object.entries(errors)) {
    if (key === "root") form.setError("root", { message });
    else form.setError(`values.${key}`, { message });
  }
}

function RecordFields({
  object,
  form,
  record,
  ownerId,
  onAnyChange,
}: {
  readonly object: CrmObject;
  readonly form: ReturnType<typeof useAppForm<RecordFormValues>>;
  readonly record?: CrmRecord;
  readonly ownerId?: string;
  readonly onAnyChange?: () => void;
}) {
  const values = form.watch("values");
  const protectedFields = form.watch("protected");
  return (
    <div className="space-y-5">
      {object.fields
        .filter((field) => !field.archived)
        .map((field) => (
          <div key={field.key} className="space-y-2">
            <CrmFieldEditor
              field={field}
              value={values[field.key]}
              links={record?.links[field.key]}
              ownerId={ownerId}
              error={fieldError(form.formState.errors, field.key)}
              onChange={(value) => {
                onAnyChange?.();
                form.setValue(`values.${field.key}`, value);
                form.clearErrors(`values.${field.key}`);
              }}
            />
            <label className="flex items-center gap-2 text-11 text-muted-foreground">
              <Checkbox
                checked={protectedFields[field.key] ?? false}
                onCheckedChange={(checked) => {
                  onAnyChange?.();
                  form.setValue(`protected.${field.key}`, checked === true);
                }}
              />
              <ShieldCheck aria-hidden="true" className="size-3" />
              Protect this field from agent changes
            </label>
          </div>
        ))}
    </div>
  );
}

function ConflictNotice({
  object,
  current,
  pending,
  onRetry,
}: {
  readonly object: CrmObject;
  readonly current: CrmRecord;
  readonly pending: boolean;
  readonly onRetry: () => void;
}) {
  return (
    <section
      className="space-y-3 rounded-lg border border-warning/30 bg-warning/10 p-3"
      aria-live="polite"
    >
      <div>
        <h3 className="text-12 font-semibold text-warning">
          This record changed elsewhere
        </h3>
        <p className="text-11 text-muted-foreground">
          The saved record is now revision {current.revision}. Review its
          current values, then retry your edits.
        </p>
      </div>
      <dl className="grid gap-1.5">
        {object.fields
          .filter((field) => !field.archived)
          .map((field) => (
            <div
              key={field.key}
              className="grid grid-cols-[minmax(5rem,0.7fr)_minmax(0,1.3fr)] gap-2 text-11"
            >
              <dt className="text-text-tertiary">{field.label}</dt>
              <dd className="min-w-0 break-words">
                {renderCrmCell(
                  field,
                  current.values[field.key],
                  current.links[field.key],
                )}
              </dd>
            </div>
          ))}
      </dl>
      <Button
        type="button"
        size="sm"
        variant="outline"
        isLoading={pending}
        onClick={onRetry}
      >
        Retry my changes
      </Button>
    </section>
  );
}

function CrmRecordEditor({
  object,
  record,
  ownerId,
  onSaved,
}: {
  readonly object: CrmObject;
  readonly record: CrmRecord;
  readonly ownerId?: string;
  readonly onSaved: () => void;
}) {
  const update = useUpdateCrmRecord(ownerId);
  const [conflict, setConflict] = useState<CrmRecord | null>(null);
  const actionKey = useRef<string | null>(null);
  const pendingPatch = useRef<RecordPatchDelta | null>(null);
  const form = useAppForm<RecordFormValues>({
    defaultValues: editorValues(object, record),
  });
  const editableFields = object.fields.filter((field) => !field.archived);

  async function save(
    expectedRevision: number,
    retainedPatch?: RecordPatchDelta,
  ) {
    form.clearErrors();
    let patch = retainedPatch;
    if (!patch) {
      const changedValueFields = editableFields.filter((field) =>
        form.getFieldState(`values.${field.key}`).isDirty,
      );
      const parsed = parseValues(
        changedValueFields,
        form.getValues("values"),
      );
      if (Object.keys(parsed.errors).length) {
        setFieldErrors(form, parsed.errors);
        return;
      }
      const protection = form.getValues("protected");
      const changedProtectionFields = editableFields.filter((field) =>
        form.getFieldState(`protected.${field.key}`).isDirty,
      );
      patch = {
        values: parsed.data,
        protect: changedProtectionFields
          .filter(
            (field) =>
              protection[field.key] && !record.overrides.includes(field.key),
          )
          .map((field) => field.key),
        unprotect: changedProtectionFields
          .filter(
            (field) =>
              !protection[field.key] && record.overrides.includes(field.key),
          )
          .map((field) => field.key),
      };
    }
    pendingPatch.current = patch;
    try {
      actionKey.current ??= crypto.randomUUID();
      await update.mutateAsync({
        id: record.id,
        expectedRevision,
        ...patch,
        idempotencyKey: actionKey.current,
      });
      actionKey.current = null;
      pendingPatch.current = null;
      setConflict(null);
      toast.success(`${object.label} saved`);
      onSaved();
    } catch (error) {
      const current = crmConflictRecord(error);
      if (current) setConflict(current);
      else {
        const errors = crmValidationErrors(error);
        if (Object.keys(errors).length) setFieldErrors(form, errors);
        else
          form.setError("root", {
            message:
              error instanceof Error
                ? error.message
                : "Could not save the record.",
          });
      }
    }
  }

  return (
    <Form {...form}>
      <form
        className="space-y-5"
        onSubmit={form.handleSubmit(() => save(record.revision))}
      >
        {conflict ? (
          <ConflictNotice
            object={object}
            current={conflict}
            pending={update.isPending}
            onRetry={() => {
              if (pendingPatch.current)
                void save(conflict.revision, pendingPatch.current);
            }}
          />
        ) : null}
        <RecordFields
          object={object}
          form={form}
          record={record}
          ownerId={ownerId}
          onAnyChange={() => {
            actionKey.current = null;
            pendingPatch.current = null;
            setConflict(null);
          }}
        />
        {form.formState.errors.root?.message ? (
          <p role="alert" className="text-11 text-destructive">
            {form.formState.errors.root.message}
          </p>
        ) : null}
        <div className="flex justify-end">
          <Button
            type="submit"
            variant="primary"
            disabled={!form.formState.isDirty}
            isLoading={update.isPending}
          >
            Save changes
          </Button>
        </div>
      </form>
    </Form>
  );
}

function RelatedRecordsGroup({
  group,
  ownerId,
  onSelectRecord,
}: {
  readonly group: CrmRelatedGroup;
  readonly ownerId?: string;
  readonly onSelectRecord?: (recordId: string) => void;
}) {
  const object = useCrmObject(group.object_key, ownerId);
  const titleField = object.data?.fields.find(
    (field) => field.type === "text" && field.required,
  );
  return (
    <div className="space-y-1.5">
      <p className="text-10 font-semibold uppercase tracking-[1.5px] text-text-tertiary">
        {object.data?.plural_label ?? group.object_key} · {group.field_key} (
        {group.count})
      </p>
      <div className="flex flex-wrap gap-1.5">
        {group.records.map((record) => (
          <Button
            key={record.id}
            type="button"
            size="sm"
            variant="outline"
            onClick={() => onSelectRecord?.(record.id)}
          >
            {titleField
              ? String(record.values[titleField.key] ?? "Untitled")
              : record.id}
          </Button>
        ))}
      </div>
    </div>
  );
}

export function CrmRecordSheet({
  recordId,
  ownerId,
  open,
  onOpenChange,
  onSelectRecord,
}: {
  readonly recordId?: string;
  readonly ownerId?: string;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onSelectRecord?: (recordId: string) => void;
}) {
  const detail = useCrmRecord(open ? recordId : undefined, ownerId);
  const object = useCrmObject(detail.data?.object_key, ownerId, open);
  const related = useCrmRelated(open ? recordId : undefined, ownerId);
  const archive = useSetCrmRecordArchived(ownerId);
  const archiveActionKey = useRef<string | null>(null);
  const data =
    detail.data && object.data
      ? { record: detail.data, object: object.data }
      : undefined;
  const titleField = data?.object.fields.find(
    (field) => field.type === "text" && field.required,
  );
  const title =
    titleField && data
      ? String(data.record.values[titleField.key] ?? data.object.label)
      : (data?.object.label ?? "Record");
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="h-full w-full overflow-y-auto sm:max-w-2xl">
        <SheetHeader className="pr-8">
          <SheetTitle className="text-15">{title}</SheetTitle>
          <SheetDescription>
            {data
              ? `${data.object.label} · revision ${data.record.revision}`
              : "Loading record..."}
          </SheetDescription>
        </SheetHeader>
        <div className="mt-6 space-y-8 pb-8">
          {detail.isPending || object.isPending ? (
            <p role="status" className="text-12 text-text-tertiary">
              Loading record...
            </p>
          ) : null}
          {detail.isError || object.isError ? (
            <p role="alert" className="text-12 text-destructive">
              Could not load this record.
            </p>
          ) : null}
          {data ? (
            <>
              {data.object.access !== "read" ? (
                <CrmRecordEditor
                  key={data.record.revision}
                  object={data.object}
                  record={data.record}
                  ownerId={ownerId}
                  onSaved={() => void detail.refetch()}
                />
              ) : (
                <section
                  className="space-y-3"
                  aria-labelledby="crm-fields-title"
                >
                  <h3 id="crm-fields-title" className="text-13 font-semibold">
                    Fields
                  </h3>
                  <dl className="space-y-2">
                    {data.object.fields
                      .filter((field) => !field.archived)
                      .map((field) => (
                        <div
                          key={field.key}
                          className="grid grid-cols-[minmax(5rem,0.7fr)_minmax(0,1.3fr)] gap-3 text-12"
                        >
                          <dt className="text-text-tertiary">{field.label}</dt>
                          <dd className="min-w-0 break-words">
                            {renderCrmCell(
                              field,
                              data.record.values[field.key],
                              data.record.links[field.key],
                            )}
                          </dd>
                        </div>
                      ))}
                  </dl>
                </section>
              )}
              <section
                className="space-y-3"
                aria-labelledby="crm-related-title"
              >
                <h3 id="crm-related-title" className="text-13 font-semibold">
                  Related records
                </h3>
                {related.isPending ? (
                  <p role="status" className="text-11 text-text-tertiary">
                    Loading related records...
                  </p>
                ) : null}
                {related.data?.groups.length === 0 ? (
                  <div className="rounded-lg border border-dashed border-border p-3 text-11 text-text-tertiary">
                    No related records.
                  </div>
                ) : null}
                {related.data?.groups.map((group) => (
                  <RelatedRecordsGroup
                    key={`${group.object_key}:${group.field_key}`}
                    group={group}
                    ownerId={ownerId}
                    onSelectRecord={onSelectRecord}
                  />
                ))}
              </section>
              <CrmActivityTimeline
                recordId={data.record.id}
                ownerId={ownerId}
                canWrite={data.object.access !== "read"}
              />
              {data.object.access !== "read" ? (
                <div className="border-t border-border/50 pt-4">
                  <Button
                    type="button"
                    variant={data.record.archived ? "outline" : "destructive"}
                    isLoading={archive.isPending}
                    onClick={async () => {
                      try {
                        archiveActionKey.current ??= crypto.randomUUID();
                        await archive.mutateAsync({
                          id: data.record.id,
                          expectedRevision: data.record.revision,
                          archived: !data.record.archived,
                          idempotencyKey: archiveActionKey.current,
                        });
                        archiveActionKey.current = null;
                        toast.success(
                          data.record.archived
                            ? "Record restored"
                            : "Record archived",
                        );
                        onOpenChange(false);
                      } catch (error) {
                        toast.error(
                          data.record.archived
                            ? "Could not restore the record"
                            : "Could not archive the record",
                          {
                            description:
                              error instanceof Error
                                ? error.message
                                : undefined,
                          },
                        );
                      }
                    }}
                  >
                    {data.record.archived ? <ArchiveRestore /> : <Archive />}
                    {data.record.archived ? "Restore record" : "Archive record"}
                  </Button>
                </div>
              ) : null}
            </>
          ) : null}
        </div>
      </SheetContent>
    </Sheet>
  );
}

export function CrmCreateRecordSheet({
  object,
  ownerId,
  open,
  onOpenChange,
  onCreated,
}: {
  readonly object: CrmObject;
  readonly ownerId?: string;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onCreated: (recordId: string) => void;
}) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="h-full w-full overflow-y-auto sm:max-w-xl">
        <SheetHeader className="pr-8">
          <SheetTitle className="text-15">
            New {object.label.toLocaleLowerCase()}
          </SheetTitle>
          <SheetDescription>Enter the record fields below.</SheetDescription>
        </SheetHeader>
        {open && object.access !== "read" ? (
          <div className="mt-6 pb-8">
            <CrmRecordForm
              key={object.key}
              object={object}
              ownerId={ownerId}
              title="Record details"
              submitLabel={`Create ${object.label.toLocaleLowerCase()}`}
              onCreated={(record) => {
                onOpenChange(false);
                onCreated(record.id);
              }}
            />
          </div>
        ) : null}
      </SheetContent>
    </Sheet>
  );
}

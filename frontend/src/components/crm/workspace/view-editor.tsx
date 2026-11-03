import { useMemo, useRef, useState } from "react";
import { CheckCircle2, Save, ShieldCheck, X } from "lucide-react";
import { z } from "zod";
import { CrmViewErrors } from "@/components/crm/workspace/view-errors";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  useCreateCrmView,
  useUpdateCrmView,
  useValidateCrmView,
  type CrmSavedView,
} from "@/hooks/use-crm-views";
import { ApiError } from "@/lib/api-client";
import {
  crmValidationErrorSchema,
  type CrmValidationError,
  type CrmViewEnvelope,
} from "@/lib/crm/views/schema";
import { validateCrmViewEnvelope } from "@/lib/crm/views/validation";
import type { CrmViewObjectMetadata } from "@/lib/crm/views/validation";

const validationDetailsSchema = z.object({
  errors: z.array(crmValidationErrorSchema),
});

const DEFAULT_CRM_VIEW_ENVELOPE: CrmViewEnvelope = {
  formatVersion: 1,
  renderer: "json-render",
  rendererSpecVersion: "nyxid-crm-v1",
  queries: {},
  spec: {
    root: "main",
    elements: {
      main: {
        type: "Stack",
        props: { gap: "md" },
        children: ["title"],
      },
      title: {
        type: "Text",
        props: { content: "New CRM view", variant: "title" },
        children: [],
      },
    },
  },
};

function readValidationErrors(cause: unknown): readonly CrmValidationError[] {
  if (!(cause instanceof ApiError)) return [];
  const parsed = validationDetailsSchema.safeParse(
    cause.errorResponse.details,
  );
  return parsed.success ? parsed.data.errors : [];
}

function readableError(cause: unknown): string {
  return cause instanceof Error ? cause.message : "Could not save this view.";
}

export function CrmViewEditor({
  view,
  ownerId,
  onSaved,
  onCancel,
  onReload,
  objects,
}: {
  readonly view?: CrmSavedView;
  readonly ownerId?: string;
  readonly onSaved: (view: CrmSavedView) => void;
  readonly onCancel: () => void;
  readonly onReload?: () => void;
  readonly objects?: readonly CrmViewObjectMetadata[];
}) {
  const initialTitle = view?.title ?? "";
  const initialJson = useMemo(
    () =>
      JSON.stringify(view?.envelope ?? DEFAULT_CRM_VIEW_ENVELOPE, null, 2),
    [view?.envelope],
  );
  const [title, setTitle] = useState(initialTitle);
  const [json, setJson] = useState(initialJson);
  const [errors, setErrors] = useState<readonly CrmValidationError[]>([]);
  const [validated, setValidated] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [requestError, setRequestError] = useState<string>();
  const actionKey = useRef<string | null>(null);
  const validateView = useValidateCrmView(ownerId);
  const createView = useCreateCrmView(ownerId);
  const updateView = useUpdateCrmView(ownerId);
  const isPending =
    validateView.isPending || createView.isPending || updateView.isPending;
  const isDirty = title !== initialTitle || json !== initialJson;

  function parseDraft(): CrmViewEnvelope | undefined {
    setValidated(false);
    setConflict(false);
    setRequestError(undefined);
    let value: unknown;
    try {
      value = JSON.parse(json) as unknown;
    } catch (cause) {
      setErrors([
        {
          path: "$",
          message:
            cause instanceof Error ? cause.message : "Envelope is not JSON",
        },
      ]);
      return undefined;
    }

    const parsed = validateCrmViewEnvelope(value, objects);
    if (!parsed.success) {
      setErrors(parsed.errors);
      return undefined;
    }
    setErrors([]);
    return parsed.data;
  }

  async function validateDraft() {
    const envelope = parseDraft();
    if (!envelope) return undefined;
    try {
      const result = await validateView.mutateAsync(envelope);
      setErrors(result.errors);
      setValidated(result.valid);
      return result.valid ? envelope : undefined;
    } catch (cause) {
      const validationErrors = readValidationErrors(cause);
      if (validationErrors.length > 0) setErrors(validationErrors);
      else setRequestError(readableError(cause));
      return undefined;
    }
  }

  async function handleSave() {
    actionKey.current ??= crypto.randomUUID();
    const idempotencyKey = actionKey.current;
    const envelope = await validateDraft();
    if (!envelope) return;
    try {
      const saved = view
        ? await updateView.mutateAsync({
            id: view.id,
            title: title.trim(),
            envelope,
            expectedRevision: view.revision,
            idempotencyKey,
          })
        : await createView.mutateAsync({
            title: title.trim(),
            envelope,
            idempotencyKey,
          });
      actionKey.current = null;
      onSaved(saved);
    } catch (cause) {
      if (
        cause instanceof ApiError &&
        cause.status === 409 &&
        cause.errorResponse.error === "crm_revision_conflict"
      ) {
        setConflict(true);
        return;
      }
      const validationErrors = readValidationErrors(cause);
      if (validationErrors.length > 0) setErrors(validationErrors);
      else setRequestError(readableError(cause));
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className="text-15 font-semibold">
            {view ? "Edit view" : "Create view"}
          </h3>
          {view ? (
            <p className="mt-1 text-11 text-text-tertiary">
              Revision {view.revision.toLocaleString()}
            </p>
          ) : null}
        </div>
        <Button
          type="button"
          size="icon"
          variant="ghost"
          onClick={onCancel}
          aria-label="Close editor"
          title="Close editor"
        >
          <X />
        </Button>
      </div>

      <div className="space-y-2">
        <Label htmlFor="crm-view-title">View name</Label>
        <Input
          id="crm-view-title"
          value={title}
          maxLength={200}
          onChange={(event) => {
            actionKey.current = null;
            setTitle(event.target.value);
            setValidated(false);
          }}
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor="crm-view-json">JSON envelope</Label>
        <Textarea
          id="crm-view-json"
          value={json}
          spellCheck={false}
          onChange={(event) => {
            actionKey.current = null;
            setJson(event.target.value);
            setValidated(false);
          }}
          className="min-h-80 resize-y font-mono text-12 leading-relaxed"
        />
      </div>

      {errors.length > 0 ? <CrmViewErrors errors={errors} /> : null}
      {validated && errors.length === 0 ? (
        <div
          role="status"
          className="flex items-center gap-2 rounded-lg border border-success/20 bg-success/[0.04] px-3 py-2 text-12 text-success"
        >
          <CheckCircle2 className="size-3.5" />
          View is valid.
        </div>
      ) : null}
      {conflict ? (
        <div
          role="alert"
          className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-warning/25 bg-warning/[0.04] px-3 py-2"
        >
          <p className="text-12 text-warning">
            This view changed after the editor opened.
          </p>
          {onReload ? (
            <Button type="button" size="sm" variant="outline" onClick={onReload}>
              Reload current view
            </Button>
          ) : null}
        </div>
      ) : null}
      {requestError ? (
        <p role="alert" className="text-12 text-destructive">
          {requestError}
        </p>
      ) : null}

      <div className="flex flex-wrap justify-end gap-2">
        <Button type="button" variant="outline" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          type="button"
          variant="secondary"
          onClick={() => void validateDraft()}
          disabled={isPending}
        >
          <ShieldCheck />
          Validate
        </Button>
        <Button
          type="button"
          variant="primary"
          onClick={() => void handleSave()}
          isLoading={isPending}
          disabled={isPending || !isDirty || title.trim().length === 0}
        >
          <ButtonIcon variant="primary">
            <Save />
          </ButtonIcon>
          Save view
        </Button>
      </div>
    </div>
  );
}

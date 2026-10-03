import { useEffect, useRef } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Form, FormSubmitErrors, useAppForm } from "@/components/ui/form";
import {
  useUploadRetention,
  useUpdateUploadRetention,
} from "@/hooks/use-upload-retention";
import {
  uploadRetentionPolicySchema,
  type UploadRetentionPolicy,
  type UploadRetentionResponse,
} from "@/schemas/upload-retention";

export function AdminUploadRetentionPage() {
  const query = useUploadRetention();
  return (
    <div className="space-y-6">
      <PageHeader
        title="Upload retention"
        description="Choose how long NyxID keeps assistant documents and images."
      />
      {query.isPending ? (
        <p role="status" className="text-xs text-muted-foreground">
          Loading retention policy…
        </p>
      ) : query.isError ? (
        <ErrorBanner
          message="Could not load retention policy."
          onRetry={() => void query.refetch()}
        />
      ) : query.data ? (
        <RetentionForm data={query.data} />
      ) : null}
    </div>
  );
}

function RetentionForm({ data }: { readonly data: UploadRetentionResponse }) {
  const update = useUpdateUploadRetention();
  const form = useAppForm<UploadRetentionPolicy>({
    resolver: zodResolver(uploadRetentionPolicySchema),
    defaultValues: data.effective,
    mode: "onChange",
  });
  const { isDirty, isValid } = form.formState;
  const baseline = useRef(data.effective);
  useEffect(() => {
    // Read values synchronously: resolver-driven isDirty can lag a user edit
    // when a replica refresh arrives in the same render.
    const draft = form.getValues();
    if (
      (Object.keys(baseline.current) as (keyof UploadRetentionPolicy)[]).every(
        (key) => Object.is(draft[key], baseline.current[key]),
      )
    ) {
      baseline.current = data.effective;
      form.reset(data.effective);
    }
  }, [data.effective, form]);
  function reset(policy: UploadRetentionPolicy) {
    baseline.current = policy;
    form.reset(policy);
  }
  const afterTurn = form.watch("images_delete_after_turn");
  const toolDays = form.watch("tool_image_days");
  async function save(policy: UploadRetentionPolicy | null) {
    try {
      const saved = await update.mutateAsync(policy);
      reset(saved.effective);
      toast.success(
        policy ? "Retention policy saved" : "Default retention restored",
      );
    } catch {
      toast.error("Could not save retention policy. Try again.");
    }
  }
  const fields = [
    {
      name: "pending_hours",
      label: "Unsent uploads (hours)",
      max: 8760,
      description:
        "Measured from upload. Applies to files not yet sent in a message.",
    },
    {
      name: "image_days",
      label: "Sent images (days)",
      max: 365,
      description: "Measured from when the image was first sent.",
    },
    {
      name: "document_days",
      label: "Sent documents (days)",
      max: 365,
      description: "Deletes the original file and its extracted text together.",
    },
  ] as const;
  return (
    <Form {...form}>
      <form
        onSubmit={(event) =>
          void form.handleSubmit((policy) => save(policy))(event)
        }
        className="max-w-2xl space-y-4"
      >
        <Card className="space-y-5 p-5">
          <div className="space-y-1">
            <h2 className="text-[13px] font-semibold">Files kept by NyxID</h2>
            <p className="text-xs text-muted-foreground">
              Changes apply to existing files immediately. Expired files are
              deleted permanently; increasing a limit cannot restore them.
            </p>
          </div>
          {fields.map(({ name, label, max, description }) => (
            <div key={name} className="space-y-1.5">
              <Label htmlFor={name}>{label}</Label>
              <Input
                id={name}
                type="number"
                min={1}
                max={max}
                step={1}
                className="max-w-40"
                {...form.register(name, { valueAsNumber: true })}
              />
              <p className="text-xs text-muted-foreground">
                {description} Default: {data.defaults[name]}. Range: 1–
                {max.toLocaleString()}.
              </p>
              {form.formState.errors[name] && (
                <p role="alert" className="text-xs text-destructive">
                  Enter a whole number from 1 to {max.toLocaleString()}.
                </p>
              )}
              {name === "image_days" && (
                <div className="flex items-start justify-between gap-4 pt-2">
                  <div className="space-y-1">
                    <Label htmlFor="images-after-turn">
                      Delete images after their first turn
                    </Label>
                    <p className="text-xs text-muted-foreground">
                      Delete as soon as the first turn using an image settles,
                      including stopped or failed turns. The day limit still
                      applies. Default: off.
                    </p>
                  </div>
                  <Switch
                    id="images-after-turn"
                    checked={afterTurn}
                    onCheckedChange={(value) =>
                      form.setValue("images_delete_after_turn", value)
                    }
                  />
                </div>
              )}
            </div>
          ))}
          <div className="space-y-2">
            <div className="flex items-start justify-between gap-4">
              <div className="space-y-1">
                <Label htmlFor="keep-tool-images">
                  Keep tool images with the conversation
                </Label>
                <p className="text-xs text-muted-foreground">
                  Default: on. Includes screenshots returned by tools.
                </p>
              </div>
              <Switch
                id="keep-tool-images"
                checked={toolDays === null}
                onCheckedChange={(keep) =>
                  form.setValue("tool_image_days", keep ? null : 30)
                }
              />
            </div>
            {toolDays !== null && (
              <div className="space-y-1.5">
                <Label htmlFor="tool-days">Tool images (days)</Label>
                <Input
                  id="tool-days"
                  className="max-w-40"
                  type="number"
                  min={1}
                  max={365}
                  step={1}
                  value={Number.isNaN(toolDays) ? "" : toolDays}
                  onChange={(e) =>
                    form.setValue("tool_image_days", e.target.valueAsNumber)
                  }
                />
                <p className="text-xs text-muted-foreground">
                  Measured from creation. Range: 1–365.
                </p>
                {form.formState.errors.tool_image_days && (
                  <p role="alert" className="text-xs text-destructive">
                    Enter a whole number from 1 to 365.
                  </p>
                )}
              </div>
            )}
          </div>
        </Card>
        <Card className="space-y-2 p-4 text-xs text-muted-foreground">
          <p>
            <strong className="font-medium text-foreground">
              NyxID’s copy only.
            </strong>{" "}
            Images an agent already saw may remain in its NyxAgent session until
            that session expires. Model providers also process those images;
            this setting does not delete their copies.
          </p>
          <p role="status">
            Policy revision {data.revision}. This replica has revision{" "}
            {data.replica_revision}
            {data.replica_revision === data.revision
              ? " (current)"
              : " (refresh pending)"}
            . Replicas refresh every {data.refresh_seconds} seconds. Attachment
            reads always check the current policy.
          </p>
          {data.replica_refreshed_at && (
            <p>
              Replica last refreshed:{" "}
              {new Date(data.replica_refreshed_at).toLocaleString()}.
            </p>
          )}
        </Card>
        <div className="flex flex-wrap items-center justify-end gap-2">
          <FormSubmitErrors />
          <Button
            type="button"
            variant="ghost"
            disabled={update.isPending || !data.overridden}
            onClick={() => void save(null)}
          >
            Restore defaults
          </Button>
          <Button
            type="button"
            variant="outline"
            disabled={!isDirty || update.isPending}
            onClick={() => reset(data.effective)}
          >
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            disabled={!isDirty || !isValid || update.isPending}
          >
            {update.isPending ? "Saving…" : "Save"}
          </Button>
        </div>
      </form>
    </Form>
  );
}

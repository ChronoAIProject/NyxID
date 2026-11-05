import { useState } from "react";
import { useCreateCrmObject } from "@/hooks/use-crm";
import { useAppForm, Form } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogTrigger,
} from "@/components/ui/dialog";

type Draft = {
  key: string;
  label: string;
  pluralLabel: string;
  pipeline: boolean;
};
const DEFAULTS: Draft = {
  key: "",
  label: "",
  pluralLabel: "",
  pipeline: false,
};
export function NewCrmObject({ ownerId }: { readonly ownerId?: string }) {
  const [open, setOpen] = useState(false);
  const form = useAppForm<Draft>({ defaultValues: DEFAULTS });
  const create = useCreateCrmObject(ownerId);
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState<{ payload: string; key: string }>();
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (
          !create.isPending &&
          (!form.formState.isDirty ||
            next ||
            window.confirm("Discard the new object draft?"))
        )
          setOpen(next);
      }}
    >
      <DialogTrigger asChild>
        <Button type="button" variant="outline">
          New object
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogTitle>New object</DialogTitle>
        <DialogDescription>
          Create a dataset with a Name field and optional status pipeline. Add
          more typed fields in its schema. Keys stay fixed. Up to 50 objects per
          workspace, including seeded objects.
        </DialogDescription>
        <Form {...form}>
          <form
            className="space-y-4"
            onSubmit={form.handleSubmit(async (draft) => {
              setError(undefined);
              const payload = JSON.stringify(draft);
              const retry =
                attempt?.payload === payload
                  ? attempt
                  : { payload, key: crypto.randomUUID() };
              setAttempt(retry);
              try {
                await create.mutateAsync({
                  idempotencyKey: retry.key,
                  input: {
                    key: draft.key,
                    label: draft.label,
                    plural_label: draft.pluralLabel,
                    fields: [
                      {
                        key: "name",
                        label: "Name",
                        type: "text",
                        max_length: 200,
                        required: true,
                        archived: false,
                      },
                      ...(draft.pipeline
                        ? [
                            {
                              key: "status",
                              label: "Status",
                              type: "select" as const,
                              required: true,
                              archived: false,
                              options: [
                                {
                                  value: "open",
                                  label: "Open",
                                  archived: false,
                                },
                                {
                                  value: "done",
                                  label: "Done",
                                  archived: false,
                                },
                              ],
                            },
                          ]
                        : []),
                    ],
                    pipeline: draft.pipeline
                      ? { stage_field: "status", closed_stages: ["done"] }
                      : null,
                  },
                });
                form.reset(DEFAULTS);
                setAttempt(undefined);
                setOpen(false);
              } catch (cause) {
                setError(
                  cause instanceof Error
                    ? cause.message
                    : "Could not create object.",
                );
              }
            })}
          >
            <fieldset disabled={create.isPending} className="space-y-4">
              <Label>
                Stable object key
                <Input
                  {...form.register("key", {
                    required: true,
                    pattern: /^[a-z][a-z0-9_]{0,47}$/,
                  })}
                  pattern="[a-z][a-z0-9_]{0,47}"
                  maxLength={48}
                  required
                />
              </Label>
              <Label>
                Label
                <Input
                  {...form.register("label", { required: true })}
                  maxLength={200}
                  required
                />
              </Label>
              <Label>
                Plural label
                <Input
                  {...form.register("pluralLabel", { required: true })}
                  maxLength={200}
                  required
                />
              </Label>
              <label className="flex gap-2 text-12">
                <input type="checkbox" {...form.register("pipeline")} />
                Include an Open / Done status pipeline
              </label>
              <Button type="submit" isLoading={create.isPending}>
                Create object
              </Button>
            </fieldset>
            {error ? (
              <p role="alert" className="text-12 text-destructive">
                {error}
              </p>
            ) : null}
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

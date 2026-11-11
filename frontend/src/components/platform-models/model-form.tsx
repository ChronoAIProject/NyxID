import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { titleCase } from "@/schemas/platform-models";
import { zodResolver } from "@hookform/resolvers/zod";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { NativeSelect } from "@/components/ui/native-select";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Form, useAppForm } from "@/components/ui/form";
import {
  adminModelPath,
  usePlatformModelChange,
} from "@/hooks/use-platform-models";
import {
  adminModelSchema,
  modelDefaults,
  modelFormSchema,
  modelPayload,
  type AdminModel,
  type ModelForm,
} from "@/schemas/platform-models";
import { EditErrors, Field, SaveError } from "./shared";

export function ModelFormEditor({
  model,
  readOnly,
  reload,
  onSaved,
  onCancel,
}: {
  readonly model?: AdminModel;
  readonly readOnly: boolean;
  readonly reload?: () => void;
  readonly onSaved?: (model: AdminModel) => void;
  readonly onCancel?: () => void;
}) {
  useUiTranslation();
  const form = useAppForm<ModelForm>({
    resolver: zodResolver(modelFormSchema),
    defaultValues: modelDefaults(model),
    mode: "onChange",
  });
  const change = usePlatformModelChange();
  async function save(values: ModelForm) {
    try {
      const result = await change.mutateAsync({
        path: model ? adminModelPath(model.id) : "/admin/platform-models",
        method: model ? "put" : "post",
        body: modelPayload(values, model?.revision),
      });
      onSaved?.(adminModelSchema.parse(result));
    } catch {
      /* shown below */
    }
  }
  return (
    <Form {...form}>
      <form onSubmit={form.handleSubmit(save)} className="space-y-5">
        <fieldset disabled={readOnly || change.isPending} className="space-y-5">
          <div className="grid gap-4 sm:grid-cols-2">
            <Field title={translateUi("Public model ID")}>
              <Input
                {...form.register("public_id")}
                readOnly={!!model}
                placeholder={translateUi("nyxid/gemini-3-pro")}
              />
            </Field>
            <Field title={translateUi("Display name")}>
              <Input {...form.register("display_name")} />
            </Field>
            <Field title={translateUi("Family")}>
              <NativeSelect
                {...form.register("family")}
                disabled={!!model || readOnly}
              >
                {["gemini", "claude", "openai", "open_model"].map((value) => (
                  <option key={value} value={value}>
                    {titleCase(value)}
                  </option>
                ))}
              </NativeSelect>
            </Field>
            {model && (
              <Field title={translateUi("Lifecycle")}>
                <NativeSelect {...form.register("lifecycle")}>
                  {["draft", "active", "retired"].map((value) => (
                    <option key={value} value={value}>
                      {titleCase(value)}
                    </option>
                  ))}
                </NativeSelect>
              </Field>
            )}
          </div>
          <Field title={translateUi("Description")}>
            <Textarea {...form.register("description")} />
          </Field>
          {(
            [
              [
                "protocols",
                "Caller protocols",
                [
                  "openai_chat",
                  "openai_responses",
                  "anthropic_messages",
                  "gemini",
                ],
              ],
              [
                "input_modalities",
                "Input",
                ["text", "image", "audio", "video"],
              ],
              [
                "output_modalities",
                "Output",
                ["text", "image", "audio", "video"],
              ],
            ] as const
          ).map(([field, label, options]) => (
            <fieldset key={field} className="space-y-2">
              <Label asChild>
                <legend>{label}</legend>
              </Label>
              <div className="flex flex-wrap gap-4">
                {options.map((option) => (
                  <Label key={option} className="flex items-center gap-2">
                    <Checkbox
                      checked={form.watch(field).includes(option)}
                      onCheckedChange={(checked) =>
                        form.setValue(
                          field,
                          checked
                            ? [...form.getValues(field), option]
                            : form.getValues(field).filter((x) => x !== option),
                        )
                      }
                    />
                    {titleCase(option)}
                  </Label>
                ))}
              </div>
            </fieldset>
          ))}
          <div className="grid gap-4 sm:grid-cols-2">
            <Field title={translateUi("Context token limit (optional)")}>
              <Input inputMode="numeric" {...form.register("context_tokens")} />
            </Field>
            <Field title={translateUi("Output token limit (optional)")}>
              <Input inputMode="numeric" {...form.register("output_tokens")} />
            </Field>
          </div>
          <div className="grid gap-4 sm:grid-cols-3">
            {(["streaming", "tools", "structured_output"] as const).map(
              (key) => (
                <Field key={key} title={titleCase(key)}>
                  <NativeSelect {...form.register(key)}>
                    <option value="unknown">{translateUi("Not specified")}</option>
                    <option value="yes">{translateUi("Supported")}</option>
                    <option value="no">{translateUi("Not supported")}</option>
                  </NativeSelect>
                </Field>
              ),
            )}
          </div>
          {model && (
            <>
              <Field title={translateUi("Logical billing service ID")}>
                <Input
                  {...form.register("billing_service_id")}
                  placeholder={translateUi("Model-offering service UUID")}
                />
              </Field>
              <Label className="flex items-center gap-2">
                <Checkbox
                  checked={form.watch("enabled")}
                  disabled={!model.execution_ready && !model.enabled}
                  onCheckedChange={(checked) =>
                    form.setValue("enabled", checked === true)
                  }
                />{translateUi("Enabled for entitled people")}</Label>
              <p className="type-secondary text-muted-foreground">{translateUi("Disabling stops new use for everyone. Activation requires execution, complete pricing and fleet readiness. Retired IDs cannot be reused.")}</p>
            </>
          )}
        </fieldset>
        <SaveError error={change.error} reload={reload} />
        <EditErrors />
        {!readOnly && (
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                form.reset(modelDefaults(model));
                onCancel?.();
              }}
            >{translateUi("Cancel")}</Button>
            <Button
              type="submit"
              variant="primary"
              disabled={
                change.isPending ||
                !form.formState.isValid ||
                (!!model && !form.formState.isDirty)
              }
            >
              {model ? translateUi("Save model") : translateUi("Create disabled draft")}
            </Button>
          </div>
        )}
      </form>
    </Form>
  );
}

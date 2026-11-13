import { getFormattingLocale } from "@/lib/formatting-locale";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import { describeChanges, changeLines } from "@/lib/grouped-form-changes";
import { zodResolver } from "@hookform/resolvers/zod";
import type { z } from "zod";
import { GroupedFormActions } from "@/components/ui/grouped-form";
import { Switch } from "@/components/ui/switch";
import {
  GroupedList,
  GroupedRow,
  GroupedTextarea,
} from "@/components/ui/grouped-list";
import { useAppForm } from "@/components/ui/form";
import { useSetSkillPreload } from "@/hooks/use-agent-skills";
import { skillPreloadFormSchema } from "@/schemas/agent-skills";

export function SkillPreloadSettings({
  agentId,
  skillId,
  name,
  revision,
  paths,
  bytes,
  disabled,
}: {
  readonly agentId: string;
  readonly skillId: string;
  readonly name: string;
  readonly revision: number;
  readonly paths: readonly string[];
  readonly bytes: number;
  readonly disabled: boolean;
}) {
  useUiTranslation();
  const save = useSetSkillPreload(agentId, skillId);
  const form = useAppForm<z.infer<typeof skillPreloadFormSchema>>({
    resolver: zodResolver(skillPreloadFormSchema),
    resetOptions: { keepDirtyValues: true },
    values: {
      enabled: paths.length > 0,
      paths: paths.length ? paths.join("\n") : "SKILL.md",
    },
  });
  const enabled = form.watch("enabled");
  const submit = form.handleSubmit(async (value) => {
    try {
      await save.mutateAsync({
        expected_revision: revision,
        paths: value.enabled
          ? value.paths
              .split("\n")
              .map((path) => path.trim())
              .filter(Boolean)
          : [],
      });
      form.reset(value, { keepDirtyValues: false });
    } catch {
      /* Request error is shown in this group’s footer. */
    }
  });
  return (
    <form className="space-y-6" onSubmit={submit}>
      <GroupedList
        title={
          paths.length
            ? translateUi("Preloaded · {{value1}} bytes", { value1: bytes.toLocaleString(getFormattingLocale()) })
            : translateUi("Preload settings")
        }
        footer={
          <>
            <p>{translateUi("Include these skill files automatically in your chats, without a skill lookup by the bot. Guest chats do not receive preloaded skills. Files count toward the model’s context.")}</p>
            <p id={`preload-help-${skillId}`}>{translateUi("One path per line. Include SKILL.md; add references as needed. Up to 8 files per skill and 64 KiB across this agent. Choosing a new skill version resets preloading for that skill.")}</p>
            {form.formState.errors.paths?.message ? (
              <p
                id={`preload-error-${skillId}`}
                role="alert"
                className="text-destructive"
              >
                {form.formState.errors.paths.message}
              </p>
            ) : null}
            {save.error ? (
              <p role="alert" className="text-destructive">
                {save.error.message}
              </p>
            ) : null}
          </>
        }
      >
        <GroupedRow
          title={translateUi("Preload for every turn")}
          accessory={
            <Switch
              aria-label={translateUi("Preload {{value1}}", { value1: name })}
              checked={enabled}
              disabled={disabled || save.isPending}
              onCheckedChange={(value) => form.setValue("enabled", value)}
            />
          }
        />
        {enabled ? (
          <GroupedRow asChild className="grouped-field">
            <Label>
              <span>{translateUi("Files to preload")}</span>
              <GroupedTextarea
                aria-label={translateUi("Preload files for {{value1}}", { value1: name })}
                className="min-h-20 font-mono text-13"
                aria-invalid={!!form.formState.errors.paths}
                aria-describedby={`preload-help-${skillId}${form.formState.errors.paths ? ` preload-error-${skillId}` : ""}`}
                disabled={disabled || save.isPending}
                {...form.register("paths")}
              />
            </Label>
          </GroupedRow>
        ) : null}
      </GroupedList>
      <GroupedFormActions
          changes={() => describeChanges(form.formState.defaultValues, form.getValues(), [{ key: "enabled", label: "Preload for every turn" }, { key: "paths", label: "Files to preload", list: changeLines }])}
        label={translateUi("{{value1}} preload", { value1: name })}
        dirty={form.formState.isDirty}
        pending={save.isPending}
        disabled={disabled}
        saveLabel={translateUi("Save preload settings")}
        save={() => submit()}
        discard={() => form.reset(undefined, { keepDirtyValues: false })}
      />
    </form>
  );
}

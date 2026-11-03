import { zodResolver } from "@hookform/resolvers/zod";
import type { z } from "zod";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  GroupedList,
  GroupedRow,
  GroupedTextarea,
  GroupedSaveBar,
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
  const save = useSetSkillPreload(agentId, skillId);
  const form = useAppForm<z.infer<typeof skillPreloadFormSchema>>({
    resolver: zodResolver(skillPreloadFormSchema),
    values: {
      enabled: paths.length > 0,
      paths: paths.length ? paths.join("\n") : "SKILL.md",
    },
  });
  const enabled = form.watch("enabled");
  return (
    <details className="space-y-3">
      <summary className="cursor-pointer px-4 py-2.5 text-12 font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
        {paths.length
          ? `Preloaded · ${bytes.toLocaleString()} bytes`
          : "Preload settings"}
      </summary>
      <form
        className="mt-2 space-y-2"
        onSubmit={form.handleSubmit((value) =>
          save.mutate({
            expected_revision: revision,
            paths: value.enabled
              ? value.paths
                  .split("\n")
                  .map((path) => path.trim())
                  .filter(Boolean)
              : [],
          }),
        )}
      >
        <GroupedList title="Preloading">
          <GroupedRow asChild>
            <label className="flex items-center justify-between gap-2 text-12">
              <Switch
                aria-label={`Preload ${name}`}
                checked={enabled}
                disabled={disabled || save.isPending}
                onCheckedChange={(value) => form.setValue("enabled", value)}
              />
              Preload for every turn
            </label>
          </GroupedRow>
          <GroupedRow className="block">
            <p className="text-11 text-muted-foreground">
              Include these skill files automatically in your chats, without a
              skill lookup by the bot. Guest chats do not receive preloaded
              skills. Files count toward the model&apos;s context.
            </p>
          </GroupedRow>
          {enabled ? (
            <GroupedRow asChild className="block space-y-1">
              <label>
                <span>Files to preload</span>
                <GroupedTextarea
                  aria-label={`Preload files for ${name}`}
                  className="min-h-20 font-mono text-11"
                  aria-invalid={!!form.formState.errors.paths}
                  aria-describedby={`preload-help-${skillId}${form.formState.errors.paths ? ` preload-error-${skillId}` : ""}`}
                  disabled={disabled || save.isPending}
                  {...form.register("paths")}
                />
                <span
                  id={`preload-help-${skillId}`}
                  className="block text-11 text-text-tertiary"
                >
                  One path per line. Include SKILL.md; add references as needed.
                  Up to 8 files per skill and 64 KiB across this agent. Choosing
                  a new skill version resets preloading for that skill.
                </span>
              </label>
            </GroupedRow>
          ) : null}
        </GroupedList>
        {form.formState.errors.paths?.message ? (
          <p
            id={`preload-error-${skillId}`}
            role="alert"
            className="text-11 text-destructive"
          >
            {form.formState.errors.paths.message}
          </p>
        ) : null}
        {save.error ? (
          <p role="alert" className="text-11 text-destructive">
            {save.error.message}
          </p>
        ) : null}
        <GroupedSaveBar>
          <Button
            size="sm"
            type="submit"
            isLoading={save.isPending}
            disabled={disabled || !form.formState.isDirty}
          >
            Save preload settings
          </Button>
        </GroupedSaveBar>
      </form>
    </details>
  );
}

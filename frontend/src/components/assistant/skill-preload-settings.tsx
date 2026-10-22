import { zodResolver } from "@hookform/resolvers/zod";
import type { z } from "zod";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
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
    <details className="border-t border-border/50 pt-2">
      <summary className="cursor-pointer text-11 font-medium">
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
        <label className="flex items-center gap-2 text-11">
          <Switch
            aria-label={`Preload ${name}`}
            checked={enabled}
            disabled={disabled || save.isPending}
            onCheckedChange={(value) => form.setValue("enabled", value)}
          />
          Preload for every turn
        </label>
        <p className="text-11 text-muted-foreground">
          Include these skill files automatically in your chats, without a skill
          lookup by the bot. Guest chats do not receive preloaded skills. Files
          count toward the model&apos;s context.
        </p>
        {enabled ? (
          <label className="block space-y-1 text-11">
            <span>Files to preload</span>
            <Textarea
              aria-label={`Preload files for ${name}`}
              className="min-h-20 font-mono text-11"
              disabled={disabled || save.isPending}
              {...form.register("paths")}
            />
            <span className="block text-text-tertiary">
              One path per line. Include SKILL.md; add references as needed. Up
              to 8 files per skill and 64 KiB across this agent. Choosing a new
              skill version resets preloading for that skill.
            </span>
          </label>
        ) : null}
        {form.formState.errors.paths?.message ? (
          <p role="alert" className="text-11 text-destructive">
            {form.formState.errors.paths.message}
          </p>
        ) : null}
        {save.error ? (
          <p role="alert" className="text-11 text-destructive">
            {save.error.message}
          </p>
        ) : null}
        <Button
          size="sm"
          type="submit"
          isLoading={save.isPending}
          disabled={disabled || !form.formState.isDirty}
        >
          Save preload settings
        </Button>
      </form>
    </details>
  );
}

import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { PageHeader } from "@/components/shared/page-header";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Form, useAppForm, FormSubmitErrors } from "@/components/ui/form";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import {
  useSystemSkills,
  useSetSystemSkills,
  useSystemSkillSearch,
  useSystemSkillVersions,
  useSystemSkillPreview,
} from "@/hooks/use-assistant-system-skills";
import {
  systemSkillsSelectionSchema,
  type SystemSkills,
  type SystemSkillsSelection,
} from "@/schemas/assistant-system-skills";

export function AdminAssistantSystemSkillsPage() {
  useUiTranslation();
  const current = useSystemSkills();
  return (
    <div className="space-y-6">
      <PageHeader
        title={translateUi("Built-in skills")}
        description={translateUi(
          "Choose up to 8 public Ornn skills for NyxBot and every specialist. Owners can disable them per agent. Skills never grant permissions.",
        )}
      />
      {current.isPending ? (
        <p role="status" className="text-12 text-muted-foreground">
          {translateUi("Loading built-in skills…")}{" "}
        </p>
      ) : current.error ? (
        <ErrorBanner
          message={current.error.message}
          onRetry={() => void current.refetch()}
        />
      ) : current.data ? (
        <SystemSkillsEditor
          key={current.data.revision}
          data={current.data}
          reload={() => void current.refetch()}
        />
      ) : null}
    </div>
  );
}

function SystemSkillsEditor({
  data,
  reload,
}: {
  readonly data: SystemSkills;
  readonly reload: () => void;
}) {
  const write = useSetSystemSkills();
  const form = useAppForm<SystemSkillsSelection>({
    resolver: zodResolver(systemSkillsSelectionSchema),
    defaultValues: {
      expected_revision: data.revision,
      skills: data.skills.map((s) => ({
        skill: s.skill_id,
        version: s.version,
      })),
    },
    mode: "onChange",
  });
  const chosen = form.watch("skills");
  const [metadata, setMetadata] = useState(data.skills);
  const [input, setInput] = useState("");
  const [query, setQuery] = useState<string | null>(null);
  const [page, setPage] = useState(1);
  const [selected, setSelected] = useState<string>();
  const [version, setVersion] = useState("");
  const [versionPage, setVersionPage] = useState(1);
  const search = useSystemSkillSearch(query, page);
  const versions = useSystemSkillVersions(selected, versionPage);
  const preview = useSystemSkillPreview(selected, version);
  const conflict = (write.error as { status?: number } | null)?.status === 409;
  const replace = (skills: SystemSkillsSelection["skills"]) =>
    form.setValue("skills", skills);
  const move = (index: number, direction: number) => {
    const next = [...chosen];
    const current = next[index];
    const adjacent = next[index + direction];
    if (!current || !adjacent) return;
    next[index] = adjacent;
    next[index + direction] = current;
    replace(next);
  };
  return (
    <Form {...form}>
      <form
        className="space-y-6"
        onSubmit={form.handleSubmit((selection) => {
          if (!write.isPending && !conflict) write.mutate(selection);
        })}
      >
        <p className="type-secondary text-text-tertiary">
          {translateUi(
            "Revision {{revision}} · {{count}}/8 skills. Changes reset affected sessions on their next turn.",
            { revision: data.revision, count: chosen.length },
          )}
        </p>
        {[write.error, search.error, versions.error, preview.error]
          .filter(Boolean)
          .map((error, i) => (
            <ErrorBanner key={i} message={error!.message} />
          ))}
        {conflict && (
          <div role="alert" className="space-y-2 text-12">
            <p>
              {translateUi(
                "System skills changed. Reload the current revision before saving again.",
              )}{" "}
            </p>
            <Button type="button" onClick={reload}>
              {translateUi("Reload current revision")}{" "}
            </Button>
          </div>
        )}
        <ol aria-label={translateUi("Built-in skills")} className="space-y-3">
          {chosen.map((item, index) => {
            const skill = metadata.find(
              (s) => s.skill_id === item.skill && s.version === item.version,
            );
            return (
              <li
                key={item.skill}
                className="space-y-2 rounded-xl border border-border/50 bg-card p-4"
              >
                <p className="type-section-heading">
                  {skill?.name ?? item.skill}{" "}
                  <span className="font-mono type-secondary text-text-tertiary">
                    {item.version}
                  </span>
                </p>
                <p className="text-12 text-muted-foreground">
                  {skill?.description}
                </p>
                <p className="type-secondary text-text-tertiary">
                  {skill?.size_bytes.toLocaleString(getFormattingLocale())}{" "}
                  {translateUi("bytes")}{" "}
                </p>
                <div className="flex gap-2">
                  <Button
                    type="button"
                    disabled={write.isPending || index === 0}
                    aria-label={translateUi("Move {{name}} up", {
                      name: skill?.name ?? item.skill,
                    })}
                    onClick={() => move(index, -1)}
                  >
                    {translateUi("Move up")}{" "}
                  </Button>
                  <Button
                    type="button"
                    disabled={write.isPending || index === chosen.length - 1}
                    aria-label={translateUi("Move {{name}} down", {
                      name: skill?.name ?? item.skill,
                    })}
                    onClick={() => move(index, 1)}
                  >
                    {translateUi("Move down")}{" "}
                  </Button>
                  <Button
                    type="button"
                    disabled={write.isPending}
                    aria-label={translateUi("Remove {{name}}", {
                      name: skill?.name ?? item.skill,
                    })}
                    onClick={() =>
                      replace(chosen.filter((_, i) => i !== index))
                    }
                  >
                    {translateUi("Remove")}{" "}
                  </Button>
                </div>
              </li>
            );
          })}
        </ol>
        {chosen.length === 0 && (
          <p className="rounded-lg border border-dashed border-border p-4 text-12 text-muted-foreground">
            {translateUi("No built-in skills selected.")}{" "}
          </p>
        )}
        <div className="space-y-3 rounded-xl border border-border/50 bg-card p-4">
          <h2 className="type-section-heading">
            {translateUi("Find an Ornn skill")}
          </h2>
          <p className="text-12 text-muted-foreground">
            {translateUi(
              "Save verifies the exact package, dependencies and public visibility with your Ornn identity.",
            )}{" "}
          </p>
          <div className="flex gap-2">
            <Input
              aria-label={translateUi("Search Ornn skills")}
              value={input}
              disabled={write.isPending}
              onChange={(event) => setInput(event.target.value)}
            />
            <Button
              type="button"
              onClick={() => {
                setQuery(input);
                setPage(1);
              }}
            >
              {translateUi("Search")}{" "}
            </Button>
          </div>
          {search.isFetching && (
            <p role="status" className="text-12">
              {translateUi("Searching…")}{" "}
            </p>
          )}
          {search.data?.items.map((item) => (
            <Button
              type="button"
              key={item.id}
              onClick={() => {
                setSelected(item.id);
                setVersion("");
                setVersionPage(1);
              }}
            >
              {item.name}
            </Button>
          ))}
          {search.data && (
            <div className="flex gap-2">
              <Button
                type="button"
                disabled={page <= 1}
                onClick={() => setPage(page - 1)}
              >
                {translateUi("Previous results")}{" "}
              </Button>
              <Button
                type="button"
                disabled={page >= search.data.total_pages}
                onClick={() => setPage(page + 1)}
              >
                {translateUi("Next results")}{" "}
              </Button>
            </div>
          )}
          {selected && (
            <div className="space-y-2">
              <Select value={version} onValueChange={setVersion}>
                <SelectTrigger aria-label={translateUi("Skill version")}>
                  <SelectValue placeholder={translateUi("Choose version")} />
                </SelectTrigger>
                <SelectContent>
                  {versions.data?.items.map((item) => (
                    <SelectItem key={item.version} value={item.version}>
                      {item.version}
                      {item.deprecated ? translateUi(" (Deprecated)") : ""}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {versions.data && (
                <div className="flex gap-2">
                  <Button
                    type="button"
                    disabled={versionPage <= 1}
                    onClick={() => setVersionPage(versionPage - 1)}
                  >
                    {translateUi("Previous versions")}{" "}
                  </Button>
                  <Button
                    type="button"
                    disabled={versionPage >= versions.data.total_pages}
                    onClick={() => setVersionPage(versionPage + 1)}
                  >
                    {translateUi("Next versions")}{" "}
                  </Button>
                </div>
              )}
              {preview.data && (
                <>
                  <p className="text-12 text-muted-foreground">
                    {preview.data.description} ·{" "}
                    {preview.data.size_bytes.toLocaleString(
                      getFormattingLocale(),
                    )}{" "}
                    {translateUi("bytes")}{" "}
                  </p>
                  <Button
                    type="button"
                    disabled={
                      write.isPending ||
                      chosen.length >= 8 ||
                      chosen.some((s) => s.skill === selected)
                    }
                    onClick={() => {
                      const skill = preview.data!;
                      replace([
                        ...chosen,
                        {
                          skill: skill.reference.skill_id,
                          version: skill.reference.version,
                        },
                      ]);
                      setMetadata([
                        ...metadata,
                        {
                          ...skill.reference,
                          description: skill.description,
                          size_bytes: skill.size_bytes,
                        },
                      ]);
                      setSelected(undefined);
                      setVersion("");
                    }}
                  >
                    {translateUi("Add skill")}{" "}
                  </Button>
                </>
              )}
            </div>
          )}
        </div>
        <FormSubmitErrors />
        <div className="flex justify-end gap-2">
          <Button
            type="button"
            disabled={write.isPending || !form.formState.isDirty}
            onClick={() => form.reset()}
          >
            {translateUi("Cancel")}{" "}
          </Button>
          <Button
            type="submit"
            variant="primary"
            isLoading={write.isPending}
            disabled={
              write.isPending ||
              !form.formState.isDirty ||
              !form.formState.isValid ||
              conflict
            }
          >
            {translateUi("Save built-in skills")}{" "}
          </Button>
        </div>
      </form>
    </Form>
  );
}

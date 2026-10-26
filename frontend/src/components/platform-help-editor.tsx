import { useEffect, useState } from "react";
import { useFieldArray } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import {
  useHelpLibraries,
  useHelpHistory,
  useHelpMutation,
  useHelpAssignment,
  useAssignHelp,
} from "@/hooks/use-platform-help";
import {
  helpContentSchema,
  type HelpContent,
  type HelpLibrary,
} from "@/schemas/platform-help";
import {
  useAppForm,
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
} from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import type { PlatformProfile } from "@/schemas/platform-onboarding";

function Preview({ content }: { content: HelpContent }) {
  return (
    <div
      className="space-y-4 rounded-lg border border-border bg-muted p-4 text-12"
      aria-label="Help preview"
    >
      {content.articles.map((a, i) => (
        <article key={i}>
          <h4 className="font-semibold">{a.title}</h4>
          <p className="whitespace-pre-wrap break-words">{a.body}</p>
          {a.tags.length > 0 && (
            <p className="text-muted-foreground">Topics: {a.tags.join(", ")}</p>
          )}
        </article>
      ))}
      {content.use_cases.map((c, i) => (
        <article key={i}>
          <h4 className="font-semibold">{c.title}</h4>
          <p className="whitespace-pre-wrap">{c.description}</p>
          <ul className="list-disc pl-5">
            {c.example_prompts.map((p, n) => (
              <li key={n}>{p}</li>
            ))}
          </ul>
        </article>
      ))}
    </div>
  );
}
function LibraryEditor({
  library,
  onDirtyChange,
}: {
  library: HelpLibrary;
  onDirtyChange: (dirty: boolean) => void;
}) {
  const [reviewed, setReviewed] = useState(library);
  const form = useAppForm({
    resolver: zodResolver(helpContentSchema),
    defaultValues: library.draft,
  });
  const articles = useFieldArray({
    control: form.control,
    name: "articles",
    keyName: "fieldKey",
  });
  const cases = useFieldArray({
    control: form.control,
    name: "use_cases",
    keyName: "fieldKey",
  });
  const mutation = useHelpMutation();
  const [preview, setPreview] = useState(false);
  const [reviewPublish, setReviewPublish] = useState(false);
  const [before, setBefore] = useState<number>();
  const history = useHelpHistory(library.id, before);
  const [rollback, setRollback] = useState<{
    revision: number;
    content: HelpContent;
  }>();
  const [notice, setNotice] = useState("");
  const changed = form.formState.isDirty;
  useEffect(() => {
    onDirtyChange(changed);
    return () => onDirtyChange(false);
  }, [changed, onDirtyChange]);
  const content = form.watch();
  const stale = library.version > reviewed.version;
  const reload = () => {
    form.reset(library.draft);
    setReviewed(library);
    setReviewPublish(false);
    setRollback(undefined);
  };
  return (
    <div className="space-y-4 text-12">
      <p>
        Published revision {library.published_revision}. Publishing updates
        every manager assigned to this library. Started replies keep their
        reviewed revision.
      </p>
      {mutation.error && <ErrorBanner message={mutation.error.message} />}
      {stale && (
        <div className="space-y-2">
          <p role="alert">
            {changed
              ? "This library changed elsewhere. Your unsaved edits are still here. Copy anything you want to keep before reloading."
              : "This library changed elsewhere. Reload to review the current draft before editing or publishing."}
          </p>
          <Button onClick={reload}>
            {changed
              ? "Discard local edits and reload"
              : "Reload current draft"}
          </Button>
        </div>
      )}
      {notice && <p role="status">{notice}</p>}
      <Form {...form}>
        <form
          className="space-y-4"
          onSubmit={form.handleSubmit((value) =>
            mutation.mutate(
              {
                action: "save",
                id: library.id,
                version: reviewed.version,
                content: value,
              },
              {
                onSuccess: (saved) => {
                  setReviewed(saved);
                  form.reset(saved.draft);
                  setNotice("Draft saved. Preview it before publishing.");
                  setReviewPublish(false);
                },
              },
            ),
          )}
        >
          <h3 className="text-13 font-semibold">FAQ articles</h3>
          {articles.fields.map((a, i) => (
            <fieldset
              key={a.fieldKey}
              className="space-y-3 rounded-lg border border-border p-3"
            >
              <legend className="px-1">Article {i + 1}</legend>
              {(["id", "title", "body"] as const).map((name) => (
                <FormField
                  key={name}
                  control={form.control}
                  name={`articles.${i}.${name}`}
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        {name === "id"
                          ? "Stable article ID"
                          : name === "title"
                            ? "Article title"
                            : "Article text"}
                      </FormLabel>
                      <FormControl>
                        {name === "body" ? (
                          <Textarea {...field} rows={5} />
                        ) : (
                          <Input {...field} />
                        )}
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              ))}
              <FormField
                control={form.control}
                name={`articles.${i}.tags`}
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Topics (comma separated)</FormLabel>
                    <FormControl>
                      <Input
                        value={field.value.join(", ")}
                        onChange={(e) =>
                          field.onChange(
                            e.target.value.trim()
                              ? e.target.value.split(",").map((s) => s.trim())
                              : [],
                          )
                        }
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <Button
                type="button"
                variant="destructive"
                disabled={articles.fields.length <= 1}
                onClick={() => articles.remove(i)}
              >
                Remove article {i + 1}
              </Button>
            </fieldset>
          ))}
          <Button
            type="button"
            disabled={articles.fields.length >= 30}
            onClick={() =>
              articles.append({ id: "", title: "", body: "", tags: [] })
            }
          >
            Add article
          </Button>
          <h3 className="text-13 font-semibold">Use cases</h3>
          <p className="text-muted-foreground">
            These describe ideas and example prompts. They do not create bots or
            grant access.
          </p>
          {cases.fields.map((c, i) => (
            <fieldset
              key={c.fieldKey}
              className="space-y-3 rounded-lg border border-border p-3"
            >
              <legend className="px-1">Use case {i + 1}</legend>
              {(["id", "title", "description"] as const).map((name) => (
                <FormField
                  key={name}
                  control={form.control}
                  name={`use_cases.${i}.${name}`}
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        {name === "id"
                          ? "Stable use-case ID"
                          : name === "title"
                            ? "Use-case title"
                            : "Description"}
                      </FormLabel>
                      <FormControl>
                        {name === "description" ? (
                          <Textarea {...field} rows={3} />
                        ) : (
                          <Input {...field} />
                        )}
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              ))}
              <FormField
                control={form.control}
                name={`use_cases.${i}.example_prompts`}
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Example prompts (one per line)</FormLabel>
                    <FormControl>
                      <Textarea
                        value={field.value.join("\n")}
                        onChange={(e) =>
                          field.onChange(
                            e.target.value ? e.target.value.split("\n") : [],
                          )
                        }
                        rows={3}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <Button
                type="button"
                variant="destructive"
                onClick={() => cases.remove(i)}
              >
                Remove use case {i + 1}
              </Button>
            </fieldset>
          ))}
          <Button
            type="button"
            disabled={cases.fields.length >= 20}
            onClick={() =>
              cases.append({
                id: "",
                title: "",
                description: "",
                example_prompts: [],
              })
            }
          >
            Add use case
          </Button>
          {form.formState.errors.root?.message && (
            <ErrorBanner message={form.formState.errors.root.message} />
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              type="submit"
              variant="primary"
              disabled={!changed || stale}
              isLoading={mutation.isPending}
            >
              Save draft
            </Button>
            <Button type="button" onClick={() => setPreview((v) => !v)}>
              Preview draft
            </Button>
            {changed && (
              <Button type="button" onClick={reload}>
                Discard unsaved edits
              </Button>
            )}
            <Button
              type="button"
              disabled={changed || stale || mutation.isPending}
              onClick={() => {
                setReviewPublish(true);
                setRollback(undefined);
              }}
            >
              Review publication
            </Button>
          </div>
        </form>
      </Form>
      {preview && <Preview content={content} />}
      {reviewPublish && !changed && !stale && (
        <div className="space-y-3">
          <h3 className="font-semibold">
            Publish this saved draft to all assigned managers?
          </h3>
          <Preview content={reviewed.draft} />
          <Button
            variant="primary"
            isLoading={mutation.isPending}
            onClick={() =>
              mutation.mutate(
                {
                  action: "publish",
                  id: library.id,
                  version: reviewed.version,
                },
                {
                  onSuccess: (saved) => {
                    setReviewed(saved);
                    form.reset(saved.draft);
                    setReviewPublish(false);
                    setNotice("Published to all assigned managers.");
                  },
                },
              )
            }
          >
            Publish shared help
          </Button>
        </div>
      )}
      <details>
        <summary>Published history and rollback</summary>
        <div className="space-y-3 py-3">
          {history.error && (
            <ErrorBanner message="Could not load published history." />
          )}
          {history.data?.map((r) => (
            <div
              key={r.revision}
              className="flex items-center justify-between gap-3"
            >
              <span>
                Revision {r.revision} ·{" "}
                {new Date(r.published_at).toLocaleString()}
              </span>
              <Button
                disabled={changed || stale || mutation.isPending}
                onClick={() => {
                  setRollback(r);
                  setReviewPublish(false);
                }}
              >
                Review revision {r.revision}
              </Button>
            </div>
          ))}
          {history.data?.length === 50 && (
            <Button onClick={() => setBefore(history.data?.at(-1)?.revision)}>
              Older revisions
            </Button>
          )}
          {before && (
            <Button onClick={() => setBefore(undefined)}>
              Newest revisions
            </Button>
          )}
          {rollback && !changed && !stale && (
            <div className="space-y-3">
              <p>
                Restore revision {rollback.revision} as a new publication for
                every assigned manager.
              </p>
              <Preview content={rollback.content} />
              <Button
                variant="primary"
                isLoading={mutation.isPending}
                onClick={() =>
                  mutation.mutate(
                    {
                      action: "publish",
                      id: library.id,
                      version: reviewed.version,
                      rollback_revision: rollback.revision,
                    },
                    {
                      onSuccess: (saved) => {
                        setReviewed(saved);
                        form.reset(saved.draft);
                        setRollback(undefined);
                        setNotice(
                          "Reviewed revision restored as a new publication.",
                        );
                      },
                    },
                  )
                }
              >
                Restore revision {rollback.revision}
              </Button>
            </div>
          )}
        </div>
      </details>
    </div>
  );
}
const createSchema = z.object({ name: z.string().trim().min(1).max(100) });
export function PlatformHelpEditor() {
  const libraries = useHelpLibraries();
  const mutation = useHelpMutation();
  const [selected, setSelected] = useState<string>();
  const [editing, setEditing] = useState(false);
  const form = useAppForm({
    resolver: zodResolver(createSchema),
    defaultValues: { name: "" },
  });
  const current =
    libraries.data?.find((l) => l.id === selected) ?? libraries.data?.[0];
  return (
    <DetailSection title="Shared FAQ and use cases">
      <div className="space-y-4 p-4">
        {libraries.isPending && <p role="status">Loading shared help…</p>}
        {libraries.error && (
          <ErrorBanner
            message="Could not load help libraries."
            onRetry={() => void libraries.refetch()}
          />
        )}
        {current && (
          <>
            <Select
              value={current.id}
              onValueChange={setSelected}
              disabled={editing}
            >
              <SelectTrigger aria-label="Help library">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {libraries.data?.map((l) => (
                  <SelectItem key={l.id} value={l.id}>
                    {l.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <LibraryEditor
              key={current.id}
              library={current}
              onDirtyChange={setEditing}
            />
          </>
        )}
        <details>
          <summary>Create another library</summary>
          <Form {...form}>
            <form
              className="space-y-3 py-3"
              onSubmit={form.handleSubmit((v) =>
                mutation.mutate(
                  { action: "create", name: v.name },
                  {
                    onSuccess: (l) => {
                      setSelected(l.id);
                      form.reset();
                    },
                  },
                ),
              )}
            >
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Library name</FormLabel>
                    <FormControl>
                      <Input {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <Button
                type="submit"
                disabled={editing}
                isLoading={mutation.isPending}
              >
                Create help library
              </Button>
              {mutation.error && (
                <ErrorBanner message={mutation.error.message} />
              )}
            </form>
          </Form>
        </details>
      </div>
    </DetailSection>
  );
}
export function PlatformHelpAssignment({
  profile,
}: {
  profile: PlatformProfile;
}) {
  const libraries = useHelpLibraries();
  const assignment = useHelpAssignment(profile.id);
  const mutation = useAssignHelp(profile.id);
  const [selected, setSelected] = useState<string>();
  if (libraries.error || assignment.error)
    return (
      <ErrorBanner message="Could not load this manager's help library." />
    );
  if (!assignment.data || !libraries.data)
    return <p role="status">Loading help assignment…</p>;
  const value = selected ?? assignment.data.library_id;
  return (
    <div className="space-y-2">
      <label className="text-12 font-medium" htmlFor={`help-${profile.id}`}>
        Shared help library
      </label>
      <Select value={value} onValueChange={setSelected}>
        <SelectTrigger id={`help-${profile.id}`}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {libraries.data.map((l) => (
            <SelectItem key={l.id} value={l.id}>
              {l.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Button
        disabled={value === assignment.data.library_id}
        isLoading={mutation.isPending}
        onClick={() =>
          mutation.mutate(
            {
              library_id: value,
              version: assignment.data!.version,
              generation: profile.generation,
            },
            { onSuccess: () => setSelected(undefined) },
          )
        }
      >
        Use this library
      </Button>
      {mutation.error && <ErrorBanner message={mutation.error.message} />}
    </div>
  );
}

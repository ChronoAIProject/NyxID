import { useUiTranslation, translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { Form, useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import {
  credentialKinds,
  credentialError,
  type ModelCredential,
  type CredentialKind,
} from "@/schemas/model-credentials";
import {
  credentialPath,
  useModelCredentials,
  useCredentialSave,
} from "@/hooks/use-model-credentials";
import { Field, EditErrors, SaveError } from "./shared";

const formSchema = z.object({
  name: z.string().trim().min(1).max(200),
  kind: z.enum(["gcp-service-account", "openai", "anthropic"]),
  credential: z.string().min(1),
});
function CredentialForm({
  record,
  close,
}: {
  readonly record?: ModelCredential;
  readonly close: () => void;
}) {
  useUiTranslation();
  const form = useAppForm<z.infer<typeof formSchema>>({
    resolver: zodResolver(formSchema),
    defaultValues: {
      name: record?.name ?? "",
      kind: record?.kind ?? "gcp-service-account",
      credential: "",
    },
    mode: "onChange",
  });
  const change = useCredentialSave();
  const [inputError, setInputError] = useState<string>();
  const kind = record ? record.kind : form.watch("kind");
  async function loadFile(file?: File) {
    if (!file) return;
    form.setValue("credential", "");
    if (!file.name.toLowerCase().endsWith(".json") || file.size > 65536) {
      setInputError("Choose a .json key file of at most 64 KiB.");
      return;
    }
    try {
      const raw = await file.text();
      const error = credentialError(raw, "gcp-service-account");
      setInputError(error);
      if (!error) form.setValue("credential", raw);
    } catch {
      setInputError("Could not read the selected key file.");
    }
  }
  return (
    <Form {...form}>
      <form
        className="space-y-4 rounded-xl border border-border/50 p-4"
        onSubmit={form.handleSubmit(async (values) => {
          const error = credentialError(values.credential, kind);
          setInputError(error);
          if (error) return;
          const body = record
            ? {
                credential: values.credential,
                expected_updated_at: record.updated_at,
              }
            : values;
          const result = await change.save(
            record
              ? `${credentialPath}/${encodeURIComponent(record.id)}`
              : credentialPath,
            body,
            !!record,
          );
          // Clear secret input after every submission, including uncertain failures.
          form.setValue("credential", "");
          if (result) close();
        })}
      >
        <h3 className="type-card-title">
          {record
            ? translateUi("Replace credential: {{name}}", { name: record.name })
            : translateUi("Add provider credential")}
        </h3>
        <fieldset disabled={change.pending} className="space-y-4">
          {!record && (
            <>
              <Field title={translateUi("Credential name")}>
                <Input {...form.register("name")} autoComplete="off" />
              </Field>
              <Field title={translateUi("Credential kind")}>
                <NativeSelect
                  {...form.register("kind")}
                  onChange={(e) => {
                    form.setValue("kind", e.target.value as CredentialKind);
                    form.setValue("credential", "");
                    setInputError(undefined);
                  }}
                >
                  {Object.entries(credentialKinds).map(([value, label]) => (
                    <option key={value} value={value}>
                      {translateUi(label)}
                    </option>
                  ))}
                </NativeSelect>
              </Field>
            </>
          )}
          {(kind === "gcp-service-account" || kind === null) && (
            <Field title={translateUi("Service account JSON file")}>
              <Input
                type="file"
                accept=".json,application/json"
                onChange={(e) => {
                  void loadFile(e.target.files?.[0]);
                  e.target.value = "";
                }}
              />
            </Field>
          )}
          <Field
            title={
              kind === "gcp-service-account"
                ? translateUi("Service account JSON (paste)")
                : translateUi("Provider credential")
            }
          >
            <Input
              type="password"
              autoComplete="new-password"
              spellCheck={false}
              {...form.register("credential")}
            />
          </Field>
          <p className="type-secondary text-muted-foreground">
            {translateUi("Encrypted on save. Existing secrets cannot be viewed. Saving does not contact the provider.")}
          </p>
        </fieldset>
        {(inputError || change.error) && (
          <p role="alert" className="type-validation-error text-destructive">
            {translateUi(inputError || change.error!)}
          </p>
        )}
        <EditErrors />
        <div className="flex justify-end gap-2">
          <Button
            type="button"
            variant="outline"
            disabled={change.pending}
            onClick={() => {
              form.reset();
              close();
            }}
          >
            {translateUi("Cancel")}
          </Button>
          <Button
            variant="primary"
            disabled={change.pending || !form.formState.isValid}
          >
            {record ? translateUi("Replace credential") : translateUi("Save credential")}
          </Button>
        </div>
      </form>
    </Form>
  );
}
export function ProviderCredentials({
  readOnly,
}: {
  readonly readOnly: boolean;
}) {
  useUiTranslation();
  const query = useModelCredentials();
  const [editing, setEditing] = useState<ModelCredential | "new">();
  const [disable, setDisable] = useState<ModelCredential>();
  const change = useCredentialSave();
  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 className="type-section-title">{translateUi("Provider credentials")}</h2>
          <p className="type-secondary text-muted-foreground">
            {translateUi("Private credentials used by platform model routes.")}
          </p>
        </div>
        {!readOnly && (
          <Button variant="primary" onClick={() => setEditing("new")}>
            {translateUi("Add credential")}
          </Button>
        )}
      </div>
      {!readOnly && editing && (
        <CredentialForm
          key={editing === "new" ? "new" : editing.id}
          record={editing === "new" ? undefined : editing}
          close={() => setEditing(undefined)}
        />
      )}
      <SaveError error={query.error} />
      {query.isLoading && <p className="type-body">{translateUi("Loading credentials…")}</p>}
      {query.data?.pages[0]?.records.length === 0 && (
        <p className="type-body text-muted-foreground">
          {translateUi("No provider credentials configured.")}
        </p>
      )}
      <div className="divide-y divide-border/30 rounded-xl border border-border/50">
        {query.data?.pages.flatMap((page) =>
          page.records.map((record) => (
            <div key={record.id} className="space-y-2 p-4">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div>
                  <h3 className="type-card-title">{record.name}</h3>
                  <p className="type-secondary text-muted-foreground">
                    {record.kind
                      ? translateUi(credentialKinds[record.kind])
                      : translateUi("Provider credential (legacy endpoint)")}{" "}
                    · {record.enabled ? translateUi("Enabled") : translateUi("Disabled")}
                  </p>
                </div>
                {!readOnly && (
                  <div className="flex gap-2">
                    <Button
                      variant="outline"
                      onClick={() => setEditing(record)}
                    >
                      {translateUi("Rotate / replace")}
                    </Button>
                    <Button
                      variant="outline"
                      disabled={
                        !record.enabled ||
                        record.references.some((r) => r.enabled)
                      }
                      onClick={() => setDisable(record)}
                    >
                      {translateUi("Disable")}
                    </Button>
                  </div>
                )}
              </div>
              <p className="type-caption text-muted-foreground">
                {translateUi("Created {{created}} · Updated {{updated}}", { created: new Date(record.created_at).toLocaleString(getFormattingLocale()), updated: new Date(record.updated_at).toLocaleString(getFormattingLocale()) })}
              </p>
              <p className="type-secondary">
                {record.references.length
                  ? record.references
                      .map(
                        (r) =>
                          translateUi("{{name}}{{route}} ({{status}})", { name: r.display_name, route: r.route_id ? translateUi(" / route {{id}}", { id: r.route_id }) : "", status: r.enabled ? translateUi("Enabled") : translateUi("Disabled") }),
                      )
                      .join(", ")
                  : translateUi("No current routes reference this credential.")}
              </p>
              {record.references.some((r) => r.enabled) && (
                <p className="type-secondary text-muted-foreground">
                  {translateUi("Disable the referencing routes before disabling this credential.")}
                </p>
              )}
            </div>
          )),
        )}
      </div>
      {query.hasNextPage && (
        <Button
          variant="outline"
          disabled={query.isFetchingNextPage}
          onClick={() => void query.fetchNextPage()}
        >
          {translateUi("Load more credentials")}
        </Button>
      )}
      {!readOnly && disable && (
        <div className="space-y-3 rounded-lg border border-border p-4">
          <p className="type-body">
            {translateUi("Disable {{name}}? Routes using it will be unavailable until an administrator re-enables it.", { name: disable.name })}
          </p>
          <div className="flex gap-2">
            <Button
              variant="outline"
              disabled={change.pending}
              onClick={() => setDisable(undefined)}
            >
              {translateUi("Cancel")}
            </Button>
            <Button
              variant="destructive"
              disabled={change.pending}
              onClick={async () => {
                if (
                  await change.save(
                    `${credentialPath}/${encodeURIComponent(disable.id)}`,
                    { enabled: false, expected_updated_at: disable.updated_at },
                    true,
                  )
                )
                  setDisable(undefined);
              }}
            >
              {translateUi("Confirm disable")}
            </Button>
          </div>
        </div>
      )}
      {change.error && (
        <p role="alert" className="type-validation-error text-destructive">
          {translateUi(change.error)}
        </p>
      )}
      <Button
        variant="outline"
        onClick={() => {
          setEditing(undefined);
          setDisable(undefined);
          void query.refetch();
        }}
      >
        {translateUi("Reload credential inventory")}
      </Button>
    </section>
  );
}

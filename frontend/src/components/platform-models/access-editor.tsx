import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { titleCase } from "@/schemas/platform-models";
import { useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAdminUsers } from "@/hooks/use-admin";
import { useGroups } from "@/hooks/use-rbac";
import {
  adminModelPath,
  useModelAccess,
  usePlatformModelChange,
} from "@/hooks/use-platform-models";
import { entitlementsSchema, type AdminModel } from "@/schemas/platform-models";
import { Form, useAppForm } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { EditErrors, Field, SaveError } from "./shared";

type Kind = "person" | "org" | "group";
function GroupChoices({ choose }: { readonly choose: (id: string) => void }) {
  useUiTranslation();
  const groups = useGroups();
  return (
    <Field title={translateUi("Select group")}>
      <NativeSelect defaultValue="" onChange={(e) => choose(e.target.value)}>
        <option value="">{translateUi("Choose a group")}</option>
        {groups.data?.groups.map((g) => (
          <option key={g.id} value={g.id}>
            {g.name}
          </option>
        ))}
      </NativeSelect>
    </Field>
  );
}
function TargetPicker({
  kind,
  choose,
}: {
  readonly kind: Kind;
  readonly choose: (id: string) => void;
}) {
  useUiTranslation();
  const [search, setSearch] = useState("");
  const users = useAdminUsers(
    1,
    25,
    search,
    kind === "org" ? "org" : "person",
    { enabled: kind !== "group" && search.trim().length >= 2 },
  );
  if (kind === "group") return <GroupChoices choose={choose} />;
  return (
    <div className="space-y-2">
      <Field title={translateUi("Find {{value1}}", { value1: kind === "org" ? "organization" : "person" })}>
        <Input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={translateUi("Search by name")}
        />
      </Field>
      {users.error && <SaveError error={users.error} />}
      {users.data && (
        <Field title={translateUi("Search results")}>
          <NativeSelect
            defaultValue=""
            onChange={(e) => choose(e.target.value)}
          >
            <option value="">{translateUi("Select a result")}</option>
            {users.data.users.map((u) => (
              <option key={u.id} value={u.id}>
                {u.display_name || u.email}
              </option>
            ))}
          </NativeSelect>
        </Field>
      )}
    </div>
  );
}
export function AccessEditor({
  model,
  readOnly,
  reload,
}: {
  readonly model: AdminModel;
  readonly readOnly: boolean;
  readonly reload: () => void;
}) {
  useUiTranslation();
  const form = useAppForm<z.infer<typeof entitlementsSchema>>({
    resolver: zodResolver(entitlementsSchema),
    defaultValues: model.entitlements,
    mode: "onChange",
  });
  const change = usePlatformModelChange();
  const [kind, setKind] = useState<Kind>("person");
  const [target, setTarget] = useState("");
  const [effect, setEffect] = useState<"allow" | "deny">("allow");
  const [person, setPerson] = useState("");
  const [explainId, setExplainId] = useState("");
  const explanation = useModelAccess(model.id, explainId);
  const rules = form.watch("rules");
  return (
    <div className="space-y-8">
      <Form {...form}>
        <form
          className="space-y-4"
          onSubmit={form.handleSubmit(async (entitlements) => {
            try {
              await change.mutateAsync({
                path: `${adminModelPath(model.id)}/entitlements`,
                body: { expected_revision: model.revision, entitlements },
              });
            } catch {
              /* shown below */
            }
          })}
        >
          <p className="type-body text-muted-foreground">{translateUi("Default deny. Any matching deny wins, including over an individual allow or All people. Organizations include active members who can proxy; groups include direct members only. Administrator status does not grant model access.")}</p>
          <fieldset
            disabled={readOnly || change.isPending}
            className="space-y-4"
          >
            <Label className="flex items-center gap-2">
              <Checkbox
                checked={form.watch("all_people")}
                onCheckedChange={(checked) =>
                  form.setValue("all_people", checked === true)
                }
              />{translateUi("Allow all authenticated people")}</Label>
            <ul className="divide-y divide-border/30">
              {rules.map((rule, i) => (
                <li
                  key={`${rule.subject_kind}-${rule.subject_id}-${i}`}
                  className="flex flex-wrap items-center justify-between gap-2 py-3"
                >
                  <span className="type-body break-all">
                    {titleCase(rule.effect)} · {titleCase(rule.subject_kind)} ·{" "}
                    {rule.subject_id}
                  </span>
                  {!readOnly && (
                    <Button
                      type="button"
                      variant="ghost"
                      onClick={() =>
                        form.setValue(
                          "rules",
                          rules.filter((_, n) => n !== i),
                        )
                      }
                    >{translateUi("Remove ")}{rule.effect}{translateUi(" rule ")}{i + 1}
                    </Button>
                  )}
                </li>
              ))}
            </ul>
            {!readOnly && (
              <div className="space-y-4 rounded-xl border border-border/50 p-4">
                <div className="grid gap-4 sm:grid-cols-2">
                  <Field title={translateUi("Target type")}>
                    <NativeSelect
                      value={kind}
                      onChange={(e) => {
                        setKind(e.target.value as Kind);
                        setTarget("");
                      }}
                    >
                      <option value="person">{translateUi("Person")}</option>
                      <option value="org">{translateUi("Organization")}</option>
                      <option value="group">{translateUi("Group")}</option>
                    </NativeSelect>
                  </Field>
                  <Field title={translateUi("Effect")}>
                    <NativeSelect
                      value={effect}
                      onChange={(e) =>
                        setEffect(e.target.value as "allow" | "deny")
                      }
                    >
                      <option value="allow">{translateUi("Allow")}</option>
                      <option value="deny">{translateUi("Deny")}</option>
                    </NativeSelect>
                  </Field>
                </div>
                <TargetPicker kind={kind} choose={setTarget} />
                <Field title={translateUi("Target ID")}>
                  <Input
                    value={target}
                    onChange={(e) => setTarget(e.target.value)}
                    placeholder={translateUi("Select a result or enter a UUID")}
                  />
                </Field>
                <Button
                  type="button"
                  variant="outline"
                  disabled={
                    !z.uuid().safeParse(target).success ||
                    rules.length >= 500 ||
                    rules.some(
                      (r) => r.subject_kind === kind && r.subject_id === target,
                    )
                  }
                  onClick={() => {
                    form.setValue("rules", [
                      ...rules,
                      { subject_kind: kind, subject_id: target, effect },
                    ]);
                    setTarget("");
                  }}
                >{translateUi("Add access rule")}</Button>
              </div>
            )}
          </fieldset>
          <SaveError error={change.error} reload={reload} />
          <EditErrors />
          {!readOnly && (
            <div className="flex justify-end gap-2">
              <Button
                type="button"
                variant="outline"
                onClick={() => form.reset(model.entitlements)}
              >{translateUi("Cancel")}</Button>
              <Button
                variant="primary"
                disabled={
                  !form.formState.isDirty ||
                  !form.formState.isValid ||
                  change.isPending
                }
              >{translateUi("Save access")}</Button>
            </div>
          )}
        </form>
      </Form>
      <section className="space-y-4">
        <h3 className="type-section-heading">{translateUi("Explain access for a person")}</h3>
        <TargetPicker kind="person" choose={setPerson} />
        <Field title={translateUi("Person ID")}>
          <Input value={person} onChange={(e) => setPerson(e.target.value)} />
        </Field>
        <Button
          variant="outline"
          disabled={
            !z.uuid().safeParse(person).success || explanation.isFetching
          }
          onClick={() => {
            if (person === explainId) void explanation.refetch();
            else setExplainId(person);
          }}
        >{translateUi("Explain access")}</Button>
        <SaveError error={explanation.error} />
        {explanation.data && (
          <dl className="grid gap-2 type-body">
            <div>{translateUi("Policy: ")}{titleCase(explanation.data.policy_reason)}</div>
            <div>{translateUi("Model: ")}{titleCase(explanation.data.model_reason)}</div>
            <div>{translateUi("Feature: ")}{explanation.data.flag_enabled ? translateUi("On") : translateUi("Off")}</div>
            <div>{translateUi("Can execute: ")}{explanation.data.executable ? translateUi("Yes") : translateUi("No")}</div>
          </dl>
        )}
      </section>
    </div>
  );
}

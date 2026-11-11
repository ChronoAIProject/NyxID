import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import { describeChanges, changeLines, labelValue, recordChanges } from "@/lib/grouped-form-changes";
import { GroupedDialog as Dialog, GroupedDialogContent as DialogContent } from "@/components/ui/grouped-dialog";
import {
  GroupedFormActions,
  GroupedFormScope,
  GroupedFormBar,
} from "@/components/ui/grouped-form";
import { useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { Button } from "@/components/ui/button";
import {
  GroupedList,
  GroupedRow,
  GroupedInput,
  GroupedTextarea,
  GroupedSelectTrigger,
  GroupedDisclosure,
} from "@/components/ui/grouped-list";
import { Form, useAppForm } from "@/components/ui/form";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import { DialogHeader, DialogTitle, DialogDescription } from "@/components/ui/dialog";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAgentSharing, useSaveAgentSharing, useSharedNotes, useSaveSharedNotes, useSharedConsents, useDecideSharedConsent } from "@/hooks/use-shared-agents";
import { sharingFormSchema, type Sharing, type SharingForm, type SharedNotes } from "@/schemas/assistant-sharing";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import { useUpdateNyxBotAgent } from "@/hooks/use-nyxbot-agents";

export function ShareAgentDialog({
  agent,
}: {
  readonly agent: AssistantAgent;
}) {
  useUiTranslation();
  const enabled = useFeature(FEATURE_FLAG.SHARED_AGENTS);
  const [open, setOpen] = useState(false);
  const policy = useAgentSharing(agent.id, open);
  if (!enabled || agent.kind !== "specialist" || agent.owner_kind === "org" || agent.share_role || agent.status === "destroyed") return null;
  return (
    <>
      <GroupedList title={translateUi("Sharing")}>
        <GroupedDisclosure search={{ id: "share-agent", title: "Share agent", breadcrumb: "Overview › Sharing" }} onClick={() => setOpen(true)}>{translateUi("Share agent")}</GroupedDisclosure>
      </GroupedList>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-h-[85vh] overflow-y-auto bg-background">
          <DialogHeader>
            <DialogTitle>{translateUi("Share ")}{agent.display_name ?? agent.name}</DialogTitle>
            <DialogDescription>{translateUi("Each person's chats and notes stay private. The person chatting pays for model and service usage. Only services you explicitly share can use your connections.")}</DialogDescription>
          </DialogHeader>
          <GroupedFormScope>
            {policy.isPending ? (
              <p role="status">{translateUi("Loading sharing settings…")}</p>
            ) : null}
            {policy.error ? (
              <ErrorBanner message={policy.error.message} />
            ) : null}
            {policy.data ? (
              <SharingEditor
                key={policy.data.version}
                id={agent.id}
                policy={policy.data}
              />
            ) : null}
            {policy.data && policy.data.revision > 0 ? (
              <Notes id={agent.id} kind="shared-memory" editable />
            ) : null}
            <GroupedFormBar dialog />
          </GroupedFormScope>
        </DialogContent>
      </Dialog>
    </>
  );
}

function SharingEditor({
  id,
  policy,
}: {
  readonly id: string;
  readonly policy: Sharing;
}) {
  useUiTranslation();
  const save = useSaveAgentSharing(id);
  const form = useAppForm<SharingForm>({ mode: "onChange", resolver: zodResolver(sharingFormSchema), defaultValues: {
    enabled: policy.enabled,
    audience: policy.audience.map(({ subject_kind, subject_id, role }) => ({ subject_kind, subject_id, role })),
    shared_access: policy.shared_access,
  } });
  const audience = form.watch("audience");
  const levels = form.watch("shared_access");
  const [subject, setSubject] = useState("");
  const [kind, setKind] = useState<"person" | "org">("person");
  const [role, setRole] = useState<"user" | "manager">("user");
  const normalizedSubject = subject.trim().toLowerCase();
  const validSubject = kind === "person"
    ? z.email().safeParse(normalizedSubject).success
      && !audience.some((a) => a.email === normalizedSubject || a.subject_id === normalizedSubject)
    : z.uuid().safeParse(normalizedSubject).success
      && !audience.some((a) => a.subject_id === normalizedSubject);
  const audienceLabel = (entry: SharingForm["audience"][number]) => {
    const view = policy.audience.find((candidate) => candidate.subject_id === entry.subject_id);
    if (entry.email) return entry.email;
    if (entry.subject_kind === "org") return [view?.slug, view?.name].filter(Boolean).join(" · ") || entry.subject_id;
    return view?.email || entry.subject_id;
  };
  const submit = form.handleSubmit(async (values) => {
    try {
      await save.mutateAsync({ ...values, expected_version: policy.version });
      form.reset(values);
    } catch {
      /* Request error stays with the sharing group. */
    }
  });
  return (
    <Form {...form}>
      <form className="space-y-5" onSubmit={submit}>
        <GroupedList
          title={translateUi("Sharing")}
          footer={
            <>
              {save.error ? (
                <p role="alert" className="text-destructive">
                  {save.error.message}
                </p>
              ) : null}
              {Object.values(form.formState.errors).length ? (
                <p role="alert" className="text-destructive">{translateUi("Check the audience and service selections before saving.")}</p>
              ) : null}
              {save.isSuccess ? <p role="status">{translateUi("Sharing saved.")}</p> : null}
            </>
          }
        >
          <GroupedRow asChild>
            <Label className="flex items-center justify-between gap-3">{translateUi("Enable sharing")}<Switch
                checked={form.watch("enabled")}
                onCheckedChange={(value) => form.setValue("enabled", value)}
              />
            </Label>
          </GroupedRow>
        </GroupedList>
        <GroupedList
          title={translateUi("People and organizations")}
          footer={translateUi("Users can chat. Managers can also edit the profile, persona and published team memory. Organization access applies to active Admins and Members.")}
        >
          {audience.map((entry, index) => (
            <GroupedRow key={entry.subject_id ?? entry.email ?? index}>
              <span className="min-w-0 flex-1 break-all">
                {entry.subject_kind === "org" ? translateUi("Organization") : translateUi("Person")} ·{" "}
                {audienceLabel(entry)}
                <br />
                {entry.role === "manager" ? translateUi("Manager") : translateUi("User")}
              </span>
              <Button
                type="button"
                variant="ghost"
                onClick={() =>
                  form.setValue(
                    "audience",
                    audience.filter((_, candidate) => candidate !== index),
                  )
                }
              >{translateUi("Remove")}</Button>
            </GroupedRow>
          ))}
          <GroupedRow>
            <Select
              value={kind}
              onValueChange={(v: "person" | "org") => {
                setKind(v);
                if (v === "org") setRole("user");
              }}
            >
              <GroupedSelectTrigger aria-label={translateUi("Audience type")}>
                <SelectValue />
              </GroupedSelectTrigger>
              <SelectContent>
                <SelectItem value="person">{translateUi("Person")}</SelectItem>
                <SelectItem value="org">{translateUi("Organization")}</SelectItem>
              </SelectContent>
            </Select>
            <Select
              value={role}
              onValueChange={(v: "user" | "manager") => setRole(v)}
              disabled={kind === "org"}
            >
              <GroupedSelectTrigger aria-label={translateUi("Sharing role")}>
                <SelectValue />
              </GroupedSelectTrigger>
              <SelectContent>
                <SelectItem value="user">{translateUi("User")}</SelectItem>
                <SelectItem value="manager">{translateUi("Manager")}</SelectItem>
              </SelectContent>
            </Select>
          </GroupedRow>
          <GroupedRow asChild className="grouped-field">
            <Label className="block space-y-1">
              <span>{kind === "org" ? translateUi("Organization ID") : translateUi("Person email")}</span>
              <GroupedInput
                value={subject}
                onChange={(e) => setSubject(e.target.value)}
                placeholder={
                  kind === "org" ? translateUi("Organization UUID") : translateUi("person@example.com")
                }
                type={kind === "person" ? "email" : "text"}
              />
            </Label>
          </GroupedRow>
          <GroupedRow>
            <Button
              type="button"
              variant="ghost"
              disabled={!validSubject || audience.length >= 500}
              onClick={() => {
                form.setValue("audience", [
                  ...audience,
                  kind === "person"
                    ? { subject_kind: kind, email: normalizedSubject, role }
                    : {
                        subject_kind: kind,
                        subject_id: normalizedSubject,
                        role,
                      },
                ]);
                setSubject("");
              }}
            >{translateUi("Add to audience")}</Button>
          </GroupedRow>
        </GroupedList>
        <GroupedList
          title={translateUi("Services for shared users")}
          footer={translateUi("Withheld services never use your connection. People can choose their own connection for the same service after consent. Machine access and saved logins are unavailable.")}
        >
          {policy.services.length ? (
            policy.services.map((service) => (
              <GroupedRow key={service.id}>
                <Label htmlFor={`share-${service.id}`}>{service.name}</Label>
                <Select
                  value={levels[service.id] ?? "withheld"}
                  onValueChange={(value) => {
                    const next = { ...levels };
                    if (value === "withheld") delete next[service.id]; else next[service.id] = z.enum(["read", "use", "all"]).parse(value);
                    form.setValue("shared_access", next);
                  }}
                >
                  <GroupedSelectTrigger
                    id={`share-${service.id}`}
                    className="w-40"
                  >
                    <SelectValue />
                  </GroupedSelectTrigger>
                  <SelectContent>
                    <SelectItem value="withheld">{translateUi("Withheld")}</SelectItem>
                    <SelectItem value="read">{translateUi("Read")}</SelectItem>
                    <SelectItem value="use">{translateUi("Use")}</SelectItem>
                    <SelectItem value="all">{translateUi("All granted actions")}</SelectItem>
                  </SelectContent>
                </Select>
              </GroupedRow>
            ))
          ) : (
            <GroupedRow title={translateUi("No eligible services granted.")} />
          )}
        </GroupedList>
        <GroupedFormActions
          changes={() => [...describeChanges(form.formState.defaultValues, form.getValues(), [{ key: "enabled", label: "Sharing enabled" }, { key: "audience", label: "People and organizations", list: (value) => Array.isArray(value) ? value.map((person: SharingForm["audience"][number]) => `${audienceLabel(person)} · ${person.role === "manager" ? "Manager" : "User"}`) : [] }]), ...recordChanges(form.formState.defaultValues?.shared_access, form.getValues("shared_access"), (key) => `Service access · ${policy.services.find((service) => service.id === key)?.name ?? key}`, labelValue({ read: "Read", use: "Use", all: "All granted actions" }, "Withheld"))]}
          disabled={!form.formState.isValid}
          label={translateUi("sharing")}
          dirty={form.formState.isDirty}
          pending={save.isPending}
          saveLabel={translateUi("Save sharing")}
          save={submit}
          discard={() => form.reset()}
        />
      </form>
    </Form>
  );
}

export function SharedAgentMemory({
  agent,
}: {
  readonly agent: AssistantAgent;
}) {
  useUiTranslation();
  return (
    <div className="space-y-5">
      <Notes
        id={agent.id}
        kind="private-notes"
        editable={agent.status !== "destroyed"}
      />
      <Notes
        id={agent.id}
        kind="shared-memory"
        editable={
          agent.status !== "destroyed" &&
          (agent.share_role === "manager" || !agent.share_role)
        }
      />
    </div>
  );
}

function Notes({
  id,
  kind,
  editable,
}: {
  readonly id: string;
  readonly kind: "private-notes" | "shared-memory";
  readonly editable: boolean;
}) {
  useUiTranslation();
  const query = useSharedNotes(id, kind);
  const title =
    kind === "private-notes" ? "Your private notes" : "Published team memory";
  const footer =
    kind === "private-notes"
      ? "Only you and this agent in your private chats can read these notes. Shared chats use this agent’s pinned skills for instructions and packages. Skills do not grant services or tools: shared service levels and your connection consent still apply. Only the owner can change pinned skills."
      : "Everyone who can use this shared agent can read these notes. They are published explicitly by the owner or a manager.";
  return query.data ? (
    <NotesEditor
      key={query.data.revision}
      id={id}
      kind={kind}
      editable={editable}
      value={query.data}
      title={title}
      footer={footer}
    />
  ) : (
    <GroupedList
      title={title}
      footer={
        <>
          <p>{footer}</p>
          {query.error ? (
            <p role="alert" className="text-destructive">
              {query.error.message}
            </p>
          ) : null}
        </>
      }
    >
      <GroupedRow
        title={query.isPending ? translateUi("Loading notes…") : translateUi("Notes unavailable")}
      />
    </GroupedList>
  );
}

const notesFormSchema = z.object({ text: z.string().refine((value) => {
  const notes = value.split("\n").map((s) => s.trim()).filter(Boolean);
  return notes.length <= 50 && notes.every((s) => [...s].length <= 500);
}, "Use at most 50 notes, each at most 500 characters.") });

function NotesEditor({
  id,
  kind,
  editable,
  value,
  title,
  footer,
}: {
  readonly id: string;
  readonly kind: "private-notes" | "shared-memory";
  readonly editable: boolean;
  readonly value: SharedNotes;
  readonly title: string;
  readonly footer: string;
}) {
  useUiTranslation();
  const save = useSaveSharedNotes(id, kind);
  const form = useAppForm<z.infer<typeof notesFormSchema>>({ mode: "onChange", resolver: zodResolver(notesFormSchema), defaultValues: { text: value.notes.map((n) => n.text).join("\n") } });
  const submit = form.handleSubmit(async ({ text }) => {
    try {
      await save.mutateAsync({
        expected_revision: value.revision,
        notes: text
          .split("\n")
          .map((s) => s.trim())
          .filter(Boolean),
      });
      form.reset({ text });
    } catch {
      /* The notes footer renders the request error. */
    }
  });
  if (!editable)
    return (
      <GroupedList title={title} footer={footer}>
        <ul>
          {value.notes.length ? (
            value.notes.map((n) => (
              <GroupedRow search={{ id: `shared-note-${kind}-${n.id}`, title: n.text, breadcrumb: `Memory › ${title}` }} asChild key={n.id}>
                <li className="break-words">{n.text}</li>
              </GroupedRow>
            ))
          ) : (
            <GroupedRow title={translateUi("No notes yet")} />
          )}
        </ul>
      </GroupedList>
    );
  return (
    <Form {...form}>
      <form className="space-y-2" onSubmit={submit}>
        <GroupedList
          title={title}
          footer={
            <>
              <p>{footer}</p>
              {form.formState.errors.text ? (
                <p
                  id={`notes-error-${kind}`}
                  role="alert"
                  className="text-destructive"
                >
                  {form.formState.errors.text.message}
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
          <GroupedRow search={{ id: `shared-notes-${kind}`, title, breadcrumb: "Memory", keywords: form.watch("text") }} asChild className="grouped-field">
            <Label>
              <span>{translateUi("One note per line")}</span>
              <GroupedTextarea
                rows={4}
                aria-invalid={!!form.formState.errors.text}
                aria-describedby={`notes-error-${kind}`}
                {...form.register("text")}
              />
            </Label>
          </GroupedRow>
        </GroupedList>
        <GroupedFormActions
          changes={() => describeChanges(form.formState.defaultValues, form.getValues(), [{ key: "text", label: "Notes", list: changeLines }])}
          disabled={!form.formState.isValid}
          label={kind === "private-notes" ? translateUi("private notes") : translateUi("team memory")}
          dirty={form.formState.isDirty}
          pending={save.isPending}
          saveLabel={translateUi("Save notes")}
          save={submit}
          discard={() => form.reset()}
        />
      </form>
    </Form>
  );
}

export function SharedConsentCards({
  agentId,
  conversationId,
}: {
  readonly agentId: string;
  readonly conversationId?: string;
}) {
  useUiTranslation();
  const query = useSharedConsents(agentId);
  const decide = useDecideSharedConsent(agentId);
  const rows = query.data?.consents.filter((c) => !conversationId || (c.conversation_id === conversationId && c.decision === "pending")) ?? [];
  return (
    <div className="space-y-6">
      {query.error || decide.error || decide.isSuccess || !rows.length ? (
        <GroupedList
          title={translateUi("Connection consent")}
          footer={
            <>
              {query.error || decide.error ? (
                <p role="alert" className="text-destructive">
                  {(query.error ?? decide.error)?.message}
                </p>
              ) : null}
              {decide.isSuccess ? (
                <p role="status">{translateUi("Choice saved. Continue the chat when ready.")}</p>
              ) : null}
            </>
          }
        >
          <GroupedRow
            title={
              query.isPending
                ? translateUi("Loading connection consent…")
                : rows.length
                  ? translateUi("Connection choice")
                  : translateUi("No connection consent requests.")
            }
          />
        </GroupedList>
      ) : null}
      {rows.map((row) => (
        <GroupedList
          key={row.id}
          title={translateUi("Connection consent")}
          footer={
            <>
              <p>{translateUi("Your connection: ")}{row.connection_name ?? row.connection_id}.
                Model and metered-service usage bills you.
              </p>
              <p>{translateUi("Applies to this exact operation, or this turn’s stateless model calls. Always remembers this scope until you revoke it or the agent or connection changes. Your connection is never shared with the owner.")}</p>
              {row.audience ? <p>{row.audience}</p> : null}
            </>
          }
        >
          <GroupedRow search={{ id: `consent-${row.id}`, title: row.connection_name ?? "Use your connection", breadcrumb: "Access › Connection consent", keywords: `${row.operation} ${row.decision}` }} title={translateUi("Use your connection")} description={row.operation} />
          {row.decision === "pending" ? (
            (["once", "always", "skip"] as const).map((decision) => (
              <GroupedRow key={decision} asChild interactive>
                <button
                  type="button"
                  disabled={decide.isPending}
                  onClick={() => decide.mutate({ consent: row.id, decision })}
                >
                  {decision === "once"
                    ? translateUi("Allow once")
                    : decision === "always"
                      ? translateUi("Always")
                      : translateUi("Skip")}
                </button>
              </GroupedRow>
            ))
          ) : (
            <>
              <GroupedRow
                title={
                  row.decision === "always" ? translateUi("Always allowed") : translateUi("Allowed once")
                }
              />
              <GroupedRow asChild interactive className="text-destructive">
                <button
                  type="button"
                  disabled={decide.isPending}
                  onClick={() =>
                    decide.mutate({ consent: row.id, decision: "revoked" })
                  }
                >{translateUi("Revoke")}</button>
              </GroupedRow>
            </>
          )}
        </GroupedList>
      ))}
    </div>
  );
}

const profileSchema = z.object({
  display_name: z.string().max(40),
  description: z.string().min(1).max(2000),
  persona: z.string().max(2000),
  voice_instructions: z.string().max(2000).optional(),
});
const PROFILE_FIELDS = [
  { key: "display_name", label: "Display name" },
  { key: "description", label: "Description" },
  { key: "persona", label: "Persona" },
  { key: "voice_instructions", label: "Voice instructions" },
] as const;
export function SharedAgentProfile({
  agent,
}: {
  readonly agent: AssistantAgent;
}) {
  useUiTranslation();
  const update = useUpdateNyxBotAgent();
  const form = useAppForm<z.infer<typeof profileSchema>>({
    resolver: zodResolver(profileSchema),
    resetOptions: { keepDirtyValues: true },
    values: {
      display_name: agent.display_name ?? "",
      description: agent.description,
      persona: agent.persona ?? "",
      voice_instructions: agent.voice_instructions ?? "",
    },
  });
  if (agent.share_role !== "manager" || agent.status === "destroyed")
    return (
      <GroupedList title={translateUi("Profile")}>
        <GroupedRow search={{ id: "shared-description", title: "Description", keywords: agent.description }} title={translateUi("Description")} description={agent.description} />
        <GroupedRow search={{ id: "shared-persona", title: "Persona", keywords: agent.persona ?? "" }} title={translateUi("Persona")} description={agent.persona} />
        <GroupedRow search={{ id: "shared-voice-instructions", title: "Voice instructions", keywords: agent.voice_instructions ?? "" }} title={translateUi("Voice instructions")} description={agent.voice_instructions} />
      </GroupedList>
    );
  const submit = form.handleSubmit(async (values) => {
    try {
      await update.mutateAsync({ id: agent.id, ...values });
      form.reset(values, { keepDirtyValues: false });
    } catch {
      /* The profile footer shows the mutation error. */
    }
  });
  return (
    <Form {...form}>
      <form className="space-y-3" onSubmit={submit}>
        <GroupedList
          title={translateUi("Profile and persona")}
          footer={
            <>
              {update.error ? (
                <p role="alert" className="text-destructive">
                  {update.error.message}
                </p>
              ) : null}
              {PROFILE_FIELDS.map(({ key: field }) =>
                form.formState.errors[field] ? (
                  <p key={field} id={`shared-profile-error-${field}`} role="alert" className="text-destructive">
                    {form.formState.errors[field]?.message}
                  </p>
                ) : null,
              )}
            </>
          }
        >
          {PROFILE_FIELDS.map(
            ({ key: field, label }) => (
              <GroupedRow search={{ id: `shared-profile-${field}`, title: label, keywords: form.watch(field) }} key={field} asChild className="grouped-field">
                <Label>
                  <span>
                    {label}
                  </span>
                  <GroupedTextarea
                    aria-invalid={!!form.formState.errors[field]}
                    aria-describedby={form.formState.errors[field] ? `shared-profile-error-${field}` : undefined}
                    {...form.register(field)}
                  />
                </Label>
              </GroupedRow>
            ),
          )}
        </GroupedList>
        <GroupedFormActions
          changes={() => describeChanges(form.formState.defaultValues, form.getValues(), PROFILE_FIELDS)}
          label={translateUi("profile")}
          dirty={form.formState.isDirty}
          pending={update.isPending}
          saveLabel={translateUi("Save profile")}
          save={submit}
          discard={() => form.reset(undefined, { keepDirtyValues: false })}
        />
      </form>
    </Form>
  );
}

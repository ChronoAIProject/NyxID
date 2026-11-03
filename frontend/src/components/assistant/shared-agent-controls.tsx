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
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from "@/components/ui/dialog";
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
  const enabled = useFeature(FEATURE_FLAG.SHARED_AGENTS);
  const [open, setOpen] = useState(false);
  const policy = useAgentSharing(agent.id, open);
  if (!enabled || agent.kind !== "specialist" || agent.owner_kind === "org" || agent.share_role || agent.status === "destroyed") return null;
  return (
    <>
      <GroupedList title="Sharing">
        <GroupedDisclosure onClick={() => setOpen(true)}>
          Share agent
        </GroupedDisclosure>
      </GroupedList>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-h-[85vh] overflow-y-auto bg-background">
          <DialogHeader>
            <DialogTitle>Share {agent.display_name ?? agent.name}</DialogTitle>
            <DialogDescription>
              Each person's chats and notes stay private. The person chatting
              pays for model and service usage. Only services you explicitly
              share can use your connections.
            </DialogDescription>
          </DialogHeader>
          <GroupedFormScope>
            {policy.isPending ? (
              <p role="status">Loading sharing settings…</p>
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
            <GroupedFormBar />
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
  const save = useSaveAgentSharing(id);
  const form = useAppForm<SharingForm>({ resolver: zodResolver(sharingFormSchema), defaultValues: {
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
          title="Sharing"
          footer={
            <>
              {save.error ? (
                <p role="alert" className="text-destructive">
                  {save.error.message}
                </p>
              ) : null}
              {Object.values(form.formState.errors).length ? (
                <p role="alert" className="text-destructive">
                  Check the audience and service selections before saving.
                </p>
              ) : null}
              {save.isSuccess ? <p role="status">Sharing saved.</p> : null}
            </>
          }
        >
          <GroupedRow asChild>
            <label className="flex items-center justify-between gap-3">
              Enable sharing
              <Switch
                checked={form.watch("enabled")}
                onCheckedChange={(value) => form.setValue("enabled", value)}
              />
            </label>
          </GroupedRow>
        </GroupedList>
        <GroupedList
          title="People and organizations"
          footer="Users can chat. Managers can also edit the profile, persona and published team memory. Organization access applies to active Admins and Members."
        >
          {audience.map((entry, index) => (
            <GroupedRow key={entry.subject_id ?? entry.email ?? index}>
              <span className="min-w-0 flex-1 break-all">
                {entry.subject_kind === "org" ? "Organization" : "Person"} ·{" "}
                {audienceLabel(entry)}
                <br />
                {entry.role === "manager" ? "Manager" : "User"}
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
              >
                Remove
              </Button>
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
              <GroupedSelectTrigger aria-label="Audience type">
                <SelectValue />
              </GroupedSelectTrigger>
              <SelectContent>
                <SelectItem value="person">Person</SelectItem>
                <SelectItem value="org">Organization</SelectItem>
              </SelectContent>
            </Select>
            <Select
              value={role}
              onValueChange={(v: "user" | "manager") => setRole(v)}
              disabled={kind === "org"}
            >
              <GroupedSelectTrigger aria-label="Sharing role">
                <SelectValue />
              </GroupedSelectTrigger>
              <SelectContent>
                <SelectItem value="user">User</SelectItem>
                <SelectItem value="manager">Manager</SelectItem>
              </SelectContent>
            </Select>
          </GroupedRow>
          <GroupedRow asChild className="grouped-field">
            <label className="block space-y-1 text-12">
              <span>{kind === "org" ? "Organization ID" : "Person email"}</span>
              <GroupedInput
                value={subject}
                onChange={(e) => setSubject(e.target.value)}
                placeholder={
                  kind === "org" ? "Organization UUID" : "person@example.com"
                }
                type={kind === "person" ? "email" : "text"}
              />
            </label>
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
            >
              Add to audience
            </Button>
          </GroupedRow>
        </GroupedList>
        <GroupedList
          title="Services for shared users"
          footer="Withheld services never use your connection. People can choose their own connection for the same service after consent. Machine access and saved logins are unavailable."
        >
          {policy.services.length ? (
            policy.services.map((service) => (
              <GroupedRow key={service.id}>
                <label htmlFor={`share-${service.id}`}>{service.name}</label>
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
                    <SelectItem value="withheld">Withheld</SelectItem>
                    <SelectItem value="read">Read</SelectItem>
                    <SelectItem value="use">Use</SelectItem>
                    <SelectItem value="all">All granted actions</SelectItem>
                  </SelectContent>
                </Select>
              </GroupedRow>
            ))
          ) : (
            <GroupedRow title="No eligible services granted." />
          )}
        </GroupedList>
        <GroupedFormActions
          label="sharing"
          dirty={form.formState.isDirty}
          pending={save.isPending}
          saveLabel="Save sharing"
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
        title={query.isPending ? "Loading notes…" : "Notes unavailable"}
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
  const save = useSaveSharedNotes(id, kind);
  const form = useAppForm<z.infer<typeof notesFormSchema>>({ resolver: zodResolver(notesFormSchema), defaultValues: { text: value.notes.map((n) => n.text).join("\n") } });
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
              <GroupedRow asChild key={n.id}>
                <li className="break-words">{n.text}</li>
              </GroupedRow>
            ))
          ) : (
            <GroupedRow title="No notes yet" />
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
          <GroupedRow asChild className="grouped-field">
            <label>
              <span>One note per line</span>
              <GroupedTextarea
                rows={4}
                aria-invalid={!!form.formState.errors.text}
                aria-describedby={`notes-error-${kind}`}
                {...form.register("text")}
              />
            </label>
          </GroupedRow>
        </GroupedList>
        <GroupedFormActions
          label={kind === "private-notes" ? "private notes" : "team memory"}
          dirty={form.formState.isDirty}
          pending={save.isPending}
          saveLabel="Save notes"
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
  const query = useSharedConsents(agentId);
  const decide = useDecideSharedConsent(agentId);
  const rows = query.data?.consents.filter((c) => !conversationId || (c.conversation_id === conversationId && c.decision === "pending")) ?? [];
  return (
    <div className="space-y-6">
      {query.error || decide.error || decide.isSuccess || !rows.length ? (
        <GroupedList
          title="Connection consent"
          footer={
            <>
              {query.error || decide.error ? (
                <p role="alert" className="text-destructive">
                  {(query.error ?? decide.error)?.message}
                </p>
              ) : null}
              {decide.isSuccess ? (
                <p role="status">Choice saved. Continue the chat when ready.</p>
              ) : null}
            </>
          }
        >
          <GroupedRow
            title={
              query.isPending
                ? "Loading connection consent…"
                : rows.length
                  ? "Connection choice"
                  : "No connection consent requests."
            }
          />
        </GroupedList>
      ) : null}
      {rows.map((row) => (
        <GroupedList
          key={row.id}
          title="Connection consent"
          footer={
            <>
              <p>
                Your connection: {row.connection_name ?? row.connection_id}.
                Model and metered-service usage bills you.
              </p>
              <p>
                Applies to this exact operation, or this turn’s stateless model
                calls. Always remembers this scope until you revoke it or the
                agent or connection changes. Your connection is never shared
                with the owner.
              </p>
              {row.audience ? <p>{row.audience}</p> : null}
            </>
          }
        >
          <GroupedRow title="Use your connection" description={row.operation} />
          {row.decision === "pending" ? (
            (["once", "always", "skip"] as const).map((decision) => (
              <GroupedRow key={decision} asChild interactive>
                <button
                  type="button"
                  disabled={decide.isPending}
                  onClick={() => decide.mutate({ consent: row.id, decision })}
                >
                  {decision === "once"
                    ? "Allow once"
                    : decision === "always"
                      ? "Always"
                      : "Skip"}
                </button>
              </GroupedRow>
            ))
          ) : (
            <>
              <GroupedRow
                title={
                  row.decision === "always" ? "Always allowed" : "Allowed once"
                }
              />
              <GroupedRow asChild interactive className="text-destructive">
                <button
                  type="button"
                  disabled={decide.isPending}
                  onClick={() =>
                    decide.mutate({ consent: row.id, decision: "revoked" })
                  }
                >
                  Revoke
                </button>
              </GroupedRow>
            </>
          )}
        </GroupedList>
      ))}
    </div>
  );
}

const profileSchema = z.object({ display_name: z.string().max(40), description: z.string().min(1).max(2000), persona: z.string().max(2000) });
export function SharedAgentProfile({
  agent,
}: {
  readonly agent: AssistantAgent;
}) {
  const update = useUpdateNyxBotAgent();
  const form = useAppForm<z.infer<typeof profileSchema>>({
    resolver: zodResolver(profileSchema),
    resetOptions: { keepDirtyValues: true },
    values: {
      display_name: agent.display_name ?? "",
      description: agent.description,
      persona: agent.persona ?? "",
    },
  });
  if (agent.share_role !== "manager" || agent.status === "destroyed")
    return (
      <GroupedList title="Profile">
        <GroupedRow title="Description" description={agent.description} />
        <GroupedRow title="Persona" description={agent.persona} />
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
          title="Profile and persona"
          footer={
            <>
              {update.error ? (
                <p role="alert" className="text-destructive">
                  {update.error.message}
                </p>
              ) : null}
              {(["display_name", "description", "persona"] as const).map((field) =>
                form.formState.errors[field] ? (
                  <p key={field} id={`shared-profile-error-${field}`} role="alert" className="text-destructive">
                    {form.formState.errors[field]?.message}
                  </p>
                ) : null,
              )}
            </>
          }
        >
          {(["display_name", "description", "persona"] as const).map(
            (field) => (
              <GroupedRow key={field} asChild className="grouped-field">
                <label>
                  <span>
                    {field === "display_name"
                      ? "Display name"
                      : field === "description"
                        ? "Description"
                        : "Persona"}
                  </span>
                  <GroupedTextarea
                    aria-invalid={!!form.formState.errors[field]}
                    aria-describedby={form.formState.errors[field] ? `shared-profile-error-${field}` : undefined}
                    {...form.register(field)}
                  />
                </label>
              </GroupedRow>
            ),
          )}
        </GroupedList>
        <GroupedFormActions
          label="profile"
          dirty={form.formState.isDirty}
          pending={update.isPending}
          saveLabel="Save profile"
          save={submit}
          discard={() => form.reset(undefined, { keepDirtyValues: false })}
        />
      </form>
    </Form>
  );
}

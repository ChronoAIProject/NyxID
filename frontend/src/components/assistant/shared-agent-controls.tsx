import { useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Form, useAppForm } from "@/components/ui/form";
import { Switch } from "@/components/ui/switch";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from "@/components/ui/dialog";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAgentSharing, useSaveAgentSharing, useSharedNotes, useSaveSharedNotes, useSharedConsents, useDecideSharedConsent } from "@/hooks/use-shared-agents";
import { sharingFormSchema, type Sharing, type SharingForm, type SharedNotes } from "@/schemas/assistant-sharing";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import { useUpdateNyxBotAgent } from "@/hooks/use-nyxbot-agents";
import { TEXTAREA_CLASS } from "./nyxbot-agent-forms";

export function ShareAgentDialog({ agent }: { readonly agent: AssistantAgent }) {
  const enabled = useFeature(FEATURE_FLAG.SHARED_AGENTS);
  const [open, setOpen] = useState(false);
  const policy = useAgentSharing(agent.id, open);
  if (!enabled || agent.kind !== "specialist" || agent.owner_kind === "org" || agent.share_role || agent.status === "destroyed") return null;
  return <>
    <Button onClick={() => setOpen(true)}>Share agent</Button>
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        <DialogHeader><DialogTitle>Share {agent.display_name ?? agent.name}</DialogTitle>
          <DialogDescription>Each person's chats and notes stay private. The person chatting pays for model and service usage. Only services you explicitly share can use your connections.</DialogDescription></DialogHeader>
        {policy.isPending ? <p role="status">Loading sharing settings…</p> : null}
        {policy.error ? <ErrorBanner message={policy.error.message} /> : null}
        {policy.data ? <SharingEditor key={policy.data.version} id={agent.id} policy={policy.data} /> : null}
        {policy.data && policy.data.revision > 0 ? <Notes id={agent.id} kind="shared-memory" editable /> : null}
      </DialogContent>
    </Dialog>
  </>;
}

function SharingEditor({ id, policy }: { readonly id: string; readonly policy: Sharing }) {
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
  return <Form {...form}><form className="space-y-5" onSubmit={form.handleSubmit((values) => save.mutate({ ...values, expected_version: policy.version }))}>
    <label className="flex items-center justify-between text-13">Enable sharing<Switch checked={form.watch("enabled")} onCheckedChange={(value) => form.setValue("enabled", value)} /></label>
    <div className="space-y-2">
      <p className="text-13 font-medium">People and organizations</p>
      <p className="text-12 text-muted-foreground">Users can chat. Managers can also edit the profile, persona and published team memory. Organization access applies to active Admins and Members.</p>
      {audience.map((entry, index) => <div key={entry.subject_id ?? entry.email ?? index} className="flex items-center gap-2 rounded-lg border border-border p-2 text-12">
        <span className="min-w-0 flex-1 break-all">{entry.subject_kind === "org" ? "Organization" : "Person"} · {audienceLabel(entry)}<br />{entry.role === "manager" ? "Manager" : "User"}</span>
        <Button type="button" variant="ghost" onClick={() => form.setValue("audience", audience.filter((_, candidate) => candidate !== index))}>Remove</Button>
      </div>)}
      <div className="flex gap-2">
        <Select value={kind} onValueChange={(v: "person" | "org") => { setKind(v); if (v === "org") setRole("user"); }}><SelectTrigger aria-label="Audience type"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="person">Person</SelectItem><SelectItem value="org">Organization</SelectItem></SelectContent></Select>
        <Select value={role} onValueChange={(v: "user" | "manager") => setRole(v)} disabled={kind === "org"}><SelectTrigger aria-label="Sharing role"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="user">User</SelectItem><SelectItem value="manager">Manager</SelectItem></SelectContent></Select>
      </div>
      <label className="block space-y-1 text-12"><span>{kind === "org" ? "Organization ID" : "Person email"}</span><Input value={subject} onChange={(e) => setSubject(e.target.value)} placeholder={kind === "org" ? "Organization UUID" : "person@example.com"} type={kind === "person" ? "email" : "text"} /></label>
      <Button type="button" disabled={!validSubject || audience.length >= 500} onClick={() => { form.setValue("audience", [...audience, kind === "person" ? { subject_kind: kind, email: normalizedSubject, role } : { subject_kind: kind, subject_id: normalizedSubject, role }]); setSubject(""); }}>Add to audience</Button>
    </div>
    <div className="space-y-2">
      <p className="text-13 font-medium">Services for shared users</p>
      <p className="text-12 text-muted-foreground">Withheld services never use your connection. People can choose their own connection for the same service after consent. Machine access and saved logins are unavailable.</p>
      {policy.services.length ? policy.services.map((service) => <div key={service.id} className="flex items-center justify-between gap-3 text-12"><label htmlFor={`share-${service.id}`}>{service.name}</label>
        <Select value={levels[service.id] ?? "withheld"} onValueChange={(value) => { const next = { ...levels }; if (value === "withheld") delete next[service.id]; else next[service.id] = z.enum(["read", "use", "all"]).parse(value); form.setValue("shared_access", next); }}>
          <SelectTrigger id={`share-${service.id}`} className="w-40"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="withheld">Withheld</SelectItem><SelectItem value="read">Read</SelectItem><SelectItem value="use">Use</SelectItem><SelectItem value="all">All granted actions</SelectItem></SelectContent>
        </Select></div>) : <p className="text-12 text-muted-foreground">No eligible services granted.</p>}
    </div>
    {save.error ? <ErrorBanner message={save.error.message} /> : null}
    <Button type="submit" variant="primary" disabled={!form.formState.isDirty} isLoading={save.isPending}>Save sharing</Button>
    {save.isSuccess ? <p role="status" className="text-12 text-muted-foreground">Sharing saved.</p> : null}
  </form></Form>;
}

export function SharedAgentMemory({ agent }: { readonly agent: AssistantAgent }) {
  return <div className="space-y-5">
    <p className="text-12 text-muted-foreground">Shared chats use this agent&apos;s pinned skills for instructions and packages. Skills do not grant services or tools: shared service levels and your connection consent still apply. Only the owner can change pinned skills.</p>
    <Notes id={agent.id} kind="private-notes" editable />
    <Notes id={agent.id} kind="shared-memory" editable={agent.share_role === "manager" || !agent.share_role} />
  </div>;
}

function Notes({ id, kind, editable }: { readonly id: string; readonly kind: "private-notes" | "shared-memory"; readonly editable: boolean }) {
  const query = useSharedNotes(id, kind);
  return <section className="space-y-2"><h3 className="text-13 font-semibold">{kind === "private-notes" ? "Your private notes" : "Published team memory"}</h3>
    <p className="text-12 text-muted-foreground">{kind === "private-notes" ? "Only you and this agent in your private chats can read these notes." : "Everyone who can use this shared agent can read these notes. They are published explicitly by the owner or a manager."}</p>
    {query.error ? <ErrorBanner message={query.error.message} /> : null}
    {query.data ? <NotesEditor key={query.data.revision} id={id} kind={kind} editable={editable} value={query.data} /> : null}
  </section>;
}

const notesFormSchema = z.object({ text: z.string().refine((value) => {
  const notes = value.split("\n").map((s) => s.trim()).filter(Boolean);
  return notes.length <= 50 && notes.every((s) => [...s].length <= 500);
}, "Use at most 50 notes, each at most 500 characters.") });

function NotesEditor({ id, kind, editable, value }: { readonly id: string; readonly kind: "private-notes" | "shared-memory"; readonly editable: boolean; readonly value: SharedNotes }) {
  const save = useSaveSharedNotes(id, kind);
  const form = useAppForm<z.infer<typeof notesFormSchema>>({ resolver: zodResolver(notesFormSchema), defaultValues: { text: value.notes.map((n) => n.text).join("\n") } });
  if (!editable) return <ul className="space-y-1 text-12">{value.notes.map((n) => <li key={n.id}>{n.text}</li>)}</ul>;
  return <Form {...form}><form className="space-y-2" onSubmit={form.handleSubmit(({ text }) => save.mutate({ expected_revision: value.revision, notes: text.split("\n").map((s) => s.trim()).filter(Boolean) }))}>
    <label className="block space-y-1 text-12"><span>One note per line</span><textarea className={TEXTAREA_CLASS} rows={4} {...form.register("text")} /></label>
    {form.formState.errors.text ? <p role="alert" className="text-12 text-destructive">{form.formState.errors.text.message}</p> : null}
    {save.error ? <ErrorBanner message={save.error.message} /> : null}
    <Button type="submit" disabled={!form.formState.isDirty} isLoading={save.isPending}>Save notes</Button>
  </form></Form>;
}

export function SharedConsentCards({ agentId, conversationId }: { readonly agentId: string; readonly conversationId?: string }) {
  const query = useSharedConsents(agentId);
  const decide = useDecideSharedConsent(agentId);
  const rows = query.data?.consents.filter((c) => !conversationId || (c.conversation_id === conversationId && c.decision === "pending")) ?? [];
  return <div className="space-y-3">
    {query.error ? <ErrorBanner message={query.error.message} /> : null}
    {rows.map((row) => <section key={row.id} className="space-y-2 rounded-xl border border-border bg-card p-4 text-12">
      <h3 className="font-semibold">Use your connection</h3>
      <p>{row.operation}</p>
      <p className="break-all text-muted-foreground">Your connection: {row.connection_name ?? row.connection_id}. Model and metered-service usage bills you.</p>
      <p className="text-muted-foreground">Applies to this exact operation, or this turn's stateless model calls. Always remembers this scope until you revoke it or the agent or connection changes. Your connection is never shared with the owner.</p>
      {row.decision === "pending" ? <div className="flex flex-wrap gap-2">{(["once", "always", "skip"] as const).map((decision) => <Button key={decision} disabled={decide.isPending} onClick={() => decide.mutate({ consent: row.id, decision })}>{decision === "once" ? "Allow once" : decision === "always" ? "Always" : "Skip"}</Button>)}</div> : <div className="flex items-center justify-between gap-2"><span>{row.decision === "always" ? "Always allowed" : "Allowed once"}</span><Button disabled={decide.isPending} onClick={() => decide.mutate({ consent: row.id, decision: "revoked" })}>Revoke</Button></div>}
    </section>)}
    {decide.error ? <ErrorBanner message={decide.error.message} /> : null}
    {decide.isSuccess ? <p role="status" className="text-12 text-muted-foreground">Choice saved. Continue the chat when ready.</p> : null}
  </div>;
}

const profileSchema = z.object({ display_name: z.string().max(40), description: z.string().min(1).max(2000), persona: z.string().max(2000) });
export function SharedAgentProfile({ agent }: { readonly agent: AssistantAgent }) {
  const update = useUpdateNyxBotAgent();
  const form = useAppForm<z.infer<typeof profileSchema>>({ resolver: zodResolver(profileSchema), defaultValues: { display_name: agent.display_name ?? "", description: agent.description, persona: agent.persona ?? "" } });
  if (agent.share_role !== "manager") return <div className="space-y-2 text-12"><p>{agent.description}</p><p className="text-muted-foreground">{agent.persona}</p></div>;
  return <Form {...form}><form className="space-y-3" onSubmit={form.handleSubmit((values) => update.mutate({ id: agent.id, ...values }, { onSuccess: () => form.reset(values) }))}>
    <h3 className="text-13 font-semibold">Profile and persona</h3>
    {(["display_name", "description", "persona"] as const).map((field) => <label key={field} className="block space-y-1 text-12"><span>{field === "display_name" ? "Display name" : field === "description" ? "Description" : "Persona"}</span><textarea className={TEXTAREA_CLASS} {...form.register(field)} /></label>)}
    {update.error ? <ErrorBanner message={update.error.message} /> : null}
    <Button type="submit" disabled={!form.formState.isDirty} isLoading={update.isPending}>Save profile</Button>
  </form></Form>;
}

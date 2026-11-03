import { useState } from "react";
import { Alert, Switch, View } from "react-native";
import { useQuery } from "@tanstack/react-query";
import { z } from "zod";
import {
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
import { WorkspaceSelectionField } from "../../components/workspace/WorkspaceSelectionField";
import { collaborationApi } from "../../lib/api/collaborationApi";
import { agentsApi } from "../../lib/api/agentsApi";
import { operationPayload } from "../../lib/api/collaborationTransport";
import { useCollaborationSession } from "../groups/useCollaborationSession";
import type { AssistantAgent } from "./agentContracts";
import {
  sharingFormSchema,
  shareAudienceSchema,
  type Sharing,
  type SharingForm,
  type SharedNotes,
} from "./sharingContracts";
import {
  operationSelectionSchema,
  type AgentServiceOperations,
  type OperationSelection,
} from "./operationContracts";

type Session = ReturnType<typeof useCollaborationSession>;
const personalOwner = (agent: AssistantAgent) =>
  agent.kind === "specialist" &&
  agent.owner_kind !== "org" &&
  !agent.share_role &&
  agent.can_maintain === true;

/** Recipient controls never reuse the owner's grants, skills or private memory. */
export function AgentCollaborationPanel({ agent }: { agent: AssistantAgent }) {
  const session = useCollaborationSession("assistant:shared-agents");
  const eligible =
    agent.kind === "specialist" &&
    agent.owner_kind !== "org" &&
    agent.status !== "destroyed";
  const sharing = useQuery({
    queryKey: [...session.key, agent.id, "sharing"],
    queryFn: ({ signal }) => collaborationApi.sharing(agent.id, signal),
    enabled: session.active && eligible && session.enabled,
    retry: false,
  });
  if (session.active && session.identity.isError)
    return (
      <Card>
        <Notice
          tone="error"
          message="Agent sharing availability could not be verified."
        />
        <ActionButton
          label="Retry sharing availability"
          onPress={() => void session.identity.refetch()}
        />
      </Card>
    );
  if (!eligible || !session.readable) return null;
  const currentPolicy =
    session.enabled &&
    !sharing.isError &&
    !sharing.isFetching &&
    sharing.dataUpdatedAt >= session.activationAt
      ? sharing.data
      : undefined;
  return (
    <>
      {session.error && <Notice tone="error" message={session.error} />}
      {session.enabled && sharing.isError && (
        <Card>
          <Notice
            tone="error"
            message="Sharing settings are unavailable or access has changed."
          />
          <ActionButton
            label="Reload sharing"
            onPress={() => void sharing.refetch()}
            disabled={!session.active}
          />
        </Card>
      )}
      {session.enabled &&
        personalOwner(agent) &&
        !sharing.isError &&
        sharing.dataUpdatedAt >= session.activationAt &&
        sharing.data?.role === "owner" && (
          <SharingEditor
            key={`${session.epoch}:${agent.id}`}
            session={session}
            agent={agent}
            policy={sharing.data}
          />
        )}
      {currentPolicy?.role === "manager" && (
        <SharedProfile
          key={`${session.epoch}:${agent.id}:profile`}
          session={session}
          agent={agent}
        />
      )}
      {currentPolicy && (agent.share_role || currentPolicy.revision > 0) && (
        <>
          <Notice
            message={
              agent.share_role
                ? `Shared by ${agent.owner_name ?? "agent owner"} · ${agent.share_role}. Your chats and private notes stay private. You pay for your model and service usage. Skills do not grant service authority.`
                : "Publishing team memory is explicit. Your existing personal memories are never shared automatically."
            }
          />
          <SharedNotesEditor
            session={session}
            id={agent.id}
            kind="private-notes"
            editable
          />
          <SharedNotesEditor
            session={session}
            id={agent.id}
            kind="shared-memory"
            editable={
              currentPolicy.role === "owner" || currentPolicy.role === "manager"
            }
          />
        </>
      )}
      <SharedConsentCards session={session} id={agent.id} />
    </>
  );
}

function SharingEditor({
  session,
  agent,
  policy,
}: {
  session: Session;
  agent: AssistantAgent;
  policy: Sharing;
}) {
  const [baseline, setBaseline] = useState(policy);
  const initial = (value: Sharing): SharingForm => ({
    enabled: value.enabled,
    audience: value.audience.map(({ subject_kind, subject_id, role }) => ({
      subject_kind,
      subject_id,
      role,
    })),
    shared_access: value.shared_access,
  });
  const [value, setValue] = useState(() => initial(policy));
  const [subject, setSubject] = useState("");
  const [kind, setKind] = useState<"person" | "org">("person");
  const [role, setRole] = useState<"user" | "manager">("user");
  const [inputError, setInputError] = useState<string>();
  const stale = baseline.version !== policy.version;
  const disabled = session.busy || !session.active;
  function add() {
    const normalized = subject.trim().toLowerCase();
    const parsed = shareAudienceSchema.safeParse(
      kind === "person"
        ? { subject_kind: kind, email: normalized, role }
        : { subject_kind: kind, subject_id: normalized, role: "user" },
    );
    if (!parsed.success) {
      setInputError(parsed.error.issues[0]?.message ?? "Check the audience.");
      return;
    }
    if (value.audience.some((a) => (a.email ?? a.subject_id) === normalized)) {
      setInputError("This audience is already selected.");
      return;
    }
    setValue((old) => ({ ...old, audience: [...old.audience, parsed.data] }));
    setSubject("");
    setInputError(undefined);
  }
  function save() {
    Alert.alert(
      "Publish sharing changes?",
      "People you select can use this agent. Managers can edit its profile and published team memory. Only explicitly selected service permissions use your connections. Private chats and personal memories remain private.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Save sharing",
          onPress: () =>
            void session.run(
              async () => {
                const fresh = await collaborationApi.sharing(agent.id);
                if (!session.valid() || fresh.role !== "owner")
                  throw new Error("Only the live owner can change sharing.");
                if (fresh.version !== baseline.version)
                  throw new Error(
                    "Sharing changed elsewhere. Reload and review before publishing.",
                  );
                await collaborationApi.saveSharing(
                  agent.id,
                  baseline.version,
                  sharingFormSchema.parse(value),
                  { authorityGuard: session.guard },
                );
                return collaborationApi.sharing(agent.id);
              },
              (result) => {
                setBaseline(result);
                setValue(initial(result));
              },
            ),
        },
      ],
    );
  }
  return (
    <Card>
      <Label>Share this personal agent</Label>
      <View
        style={{
          flexDirection: "row",
          justifyContent: "space-between",
          alignItems: "center",
        }}
      >
        <Label>Enable sharing</Label>
        <Switch
          accessibilityLabel="Enable agent sharing"
          value={value.enabled}
          disabled={disabled}
          onValueChange={(enabled) => setValue((old) => ({ ...old, enabled }))}
        />
      </View>
      <Label muted>
        Users can chat. Managers can edit profile and team memory. Organizations
        receive User access for active Admins and Members.
      </Label>
      {value.audience.map((entry, index) => {
        const view = baseline.audience.find(
          (a) => a.subject_id === entry.subject_id,
        );
        return (
          <View
            key={entry.subject_id ?? entry.email ?? index}
            style={{ gap: 8 }}
          >
            <Label>
              {entry.subject_kind} ·{" "}
              {entry.email ??
                view?.email ??
                view?.name ??
                view?.slug ??
                entry.subject_id}{" "}
              · {entry.role}
            </Label>
            <ActionButton
              compact
              variant="secondary"
              label="Remove sharing audience"
              disabled={disabled}
              onPress={() =>
                setValue((old) => ({
                  ...old,
                  audience: old.audience.filter((_, i) => i !== index),
                }))
              }
            />
            {entry.subject_kind === "person" && (
              <ActionButton
                compact
                variant="secondary"
                label={
                  entry.role === "manager"
                    ? "Change to User"
                    : "Change to Manager"
                }
                disabled={disabled}
                onPress={() =>
                  setValue((old) => ({
                    ...old,
                    audience: old.audience.map((a, i) =>
                      i === index
                        ? {
                            ...a,
                            role: a.role === "manager" ? "user" : "manager",
                          }
                        : a,
                    ),
                  }))
                }
              />
            )}
          </View>
        );
      })}
      <WorkspaceSelectionField
        label="Audience type"
        title="Choose audience"
        value={[kind]}
        options={[
          { value: "person", label: "Person" },
          { value: "org", label: "Organization" },
        ]}
        onChange={([next]) => {
          setKind(next === "org" ? "org" : "person");
          setRole("user");
        }}
        disabled={disabled}
        scopeKey={`${session.epoch}:${agent.id}:audience-kind`}
      />
      {kind === "person" && (
        <WorkspaceSelectionField
          label="Sharing role"
          title="Choose role"
          value={[role]}
          options={[
            { value: "user", label: "User" },
            { value: "manager", label: "Manager" },
          ]}
          onChange={([next]) =>
            setRole(next === "manager" ? "manager" : "user")
          }
          disabled={disabled}
          scopeKey={`${session.epoch}:${agent.id}:audience-role`}
        />
      )}
      <Field
        label={kind === "person" ? "Person email" : "Organization ID"}
        value={subject}
        onChangeText={setSubject}
        keyboardType={kind === "person" ? "email-address" : "default"}
        editable={!disabled}
        autoCorrect={false}
      />
      {inputError && <Notice tone="error" message={inputError} />}
      <ActionButton
        label="Add sharing audience"
        disabled={disabled || !subject.trim() || value.audience.length >= 500}
        onPress={add}
      />
      <Label>Services for shared users</Label>
      <Label muted>
        Withheld means no owner connection. People may separately consent to
        their own connection. Machines and saved logins are unavailable.
      </Label>
      {policy.services.map((service) => (
        <WorkspaceSelectionField
          key={service.id}
          label={service.name}
          title={`${service.name} shared permissions`}
          value={[value.shared_access[service.id] ?? "withheld"]}
          options={[
            { value: "withheld", label: "Withheld" },
            { value: "read", label: "Read" },
            { value: "use", label: "Use" },
            { value: "all", label: "All granted actions" },
          ]}
          disabled={disabled}
          scopeKey={`${session.epoch}:${agent.id}:share-service:${service.id}`}
          onChange={([level]) =>
            setValue((old) => {
              const next = { ...old.shared_access };
              if (level === "withheld") delete next[service.id];
              else if (level === "read" || level === "use" || level === "all")
                next[service.id] = level;
              return { ...old, shared_access: next };
            })
          }
        />
      ))}
      {stale && (
        <Notice
          message="Sharing changed elsewhere. Your draft is kept. Reload before saving."
          tone="error"
        />
      )}
      <ActionButton
        label="Reload sharing and discard draft"
        variant="secondary"
        disabled={disabled}
        onPress={() => {
          setBaseline(policy);
          setValue(initial(policy));
        }}
      />
      <ActionButton
        label="Save agent sharing"
        disabled={
          disabled ||
          stale ||
          JSON.stringify(value) === JSON.stringify(initial(baseline))
        }
        onPress={save}
      />
    </Card>
  );
}

function SharedNotesEditor({
  session,
  id,
  kind,
  editable,
}: {
  session: Session;
  id: string;
  kind: "private-notes" | "shared-memory";
  editable: boolean;
}) {
  const query = useQuery({
    queryKey: [...session.key, id, kind],
    queryFn: ({ signal }) => collaborationApi.notes(id, kind, signal),
    enabled: session.active,
    retry: false,
  });
  return (
    <Card>
      <Label>
        {kind === "private-notes"
          ? "Your private notes"
          : "Published team memory"}
      </Label>
      <Label muted>
        {kind === "private-notes"
          ? "Only you and this agent in your private chats can read these notes."
          : "Everyone with access to this agent can read these notes. Only the owner or a manager can publish."}
      </Label>
      {query.isError ? (
        <Notice
          tone="error"
          message="Notes could not load or sharing access changed."
        />
      ) : query.data && query.dataUpdatedAt >= session.activationAt ? (
        <NotesDraft
          session={session}
          id={id}
          kind={kind}
          value={query.data}
          editable={editable}
        />
      ) : (
        <Label muted>Loading notes…</Label>
      )}
      <ActionButton
        compact
        label="Refresh notes"
        variant="secondary"
        disabled={!session.active || session.busy}
        onPress={() => void query.refetch()}
      />
    </Card>
  );
}

function NotesDraft({
  session,
  id,
  kind,
  value,
  editable,
}: {
  session: Session;
  id: string;
  kind: "private-notes" | "shared-memory";
  value: SharedNotes;
  editable: boolean;
}) {
  const [baseline, setBaseline] = useState(value);
  const [text, setText] = useState(value.notes.map((n) => n.text).join("\n"));
  const notes = text
    .split("\n")
    .map((n) => n.trim())
    .filter(Boolean);
  const valid = notes.length <= 50 && notes.every((n) => [...n].length <= 500);
  const stale = value.revision !== baseline.revision;
  const disabled = session.busy || !session.active || !editable;
  return (
    <>
      <Field
        label={
          kind === "private-notes"
            ? "Private notes, one per line"
            : "Team notes, one per line"
        }
        value={editable ? text : value.notes.map((n) => n.text).join("\n")}
        onChangeText={setText}
        multiline
        editable={!disabled}
      />
      {!valid && (
        <Notice
          tone="error"
          message="Use at most 50 notes, each at most 500 characters."
        />
      )}
      {stale && editable && (
        <Notice message="Notes changed elsewhere. Reload to review before saving your draft." />
      )}
      {editable && (
        <>
          <ActionButton
            label="Reload notes and discard draft"
            variant="secondary"
            disabled={disabled}
            onPress={() => {
              setBaseline(value);
              setText(value.notes.map((n) => n.text).join("\n"));
            }}
          />
          <ActionButton
            label={
              kind === "private-notes"
                ? "Save private notes"
                : "Publish team memory"
            }
            disabled={
              disabled ||
              stale ||
              !valid ||
              text === baseline.notes.map((n) => n.text).join("\n")
            }
            onPress={() =>
              void session.run(
                async () => {
                  if (kind === "shared-memory") {
                    const policy = await collaborationApi.sharing(id);
                    if (
                      !session.valid() ||
                      !["owner", "manager"].includes(policy.role)
                    )
                      throw new Error(
                        "Only the owner or a current manager can publish team memory.",
                      );
                  }
                  if (!session.valid())
                    throw new Error("Keep notes open to save.");
                  await collaborationApi.saveNotes(
                    id,
                    kind,
                    baseline.revision,
                    notes,
                    { authorityGuard: session.guard },
                  );
                  return collaborationApi.notes(id, kind);
                },
                (result) => {
                  setBaseline(result);
                  setText(result.notes.map((n) => n.text).join("\n"));
                },
              )
            }
          />
        </>
      )}
    </>
  );
}

export function SharedConsentCards({
  session,
  id,
}: {
  session: Session;
  id: string;
}) {
  const query = useQuery({
    queryKey: [...session.key, id, "consents"],
    queryFn: ({ signal }) => collaborationApi.consents(id, signal),
    enabled: session.active,
    retry: false,
    refetchInterval: session.active ? 5000 : false,
  });
  return (
    <Card>
      <Label>Your connection consent</Label>
      {!session.enabled && (
        <Label muted>
          Agent sharing is disabled. You can still review and revoke your
          existing connection consent.
        </Label>
      )}
      {query.isError && (
        <Notice
          tone="error"
          message="Connection consent could not load. Retry before deciding."
        />
      )}
      {!query.isError &&
        !query.isFetching &&
        query.dataUpdatedAt >= session.activationAt &&
        query.data?.consents.map((row) => (
          <View key={row.id} style={{ gap: 8 }}>
            <Label>
              {row.connection_name ?? row.connection_id} · {row.operation}
            </Label>
            <Label muted>
              {row.audience ?? "Only your private chat can see the reply."} You
              pay for your model and service usage.
            </Label>
            {row.expires_at && (
              <Label muted>
                Expires {new Date(row.expires_at).toLocaleString()}
              </Label>
            )}
            {(session.enabled && row.decision === "pending"
              ? ([
                  ["once", "Allow once"],
                  ["always", "Always"],
                  ["skip", "Skip"],
                ] as const)
              : ([["revoked", "Revoke consent"]] as const)
            ).map(([decision, label]) => (
              <ActionButton
                compact
                key={decision}
                label={label}
                disabled={
                  session.busy ||
                  !session.active ||
                  query.isError ||
                  (decision !== "revoked" &&
                    !!row.expires_at &&
                    Date.parse(row.expires_at) <= Date.now())
                }
                onPress={() =>
                  void session.run(
                    () =>
                      collaborationApi.decideConsent(row.id, decision, {
                        authorityGuard: session.guard,
                      }),
                    undefined,
                    decision === "revoked"
                      ? { feature: null, capability: false }
                      : undefined,
                  )
                }
              />
            ))}
          </View>
        ))}
      {!query.isError &&
        !query.isFetching &&
        query.dataUpdatedAt >= session.activationAt &&
        query.data?.consents.length === 0 && (
          <Label muted>No pending or active consent.</Label>
        )}
      <ActionButton
        compact
        variant="secondary"
        label="Refresh connection consent"
        disabled={!session.active || session.busy}
        onPress={() => void query.refetch()}
      />
    </Card>
  );
}

export function SharedProfile({
  session,
  agent,
}: {
  session: Session;
  agent: AssistantAgent;
}) {
  const [baseline, setBaseline] = useState(agent);
  const [name, setName] = useState(agent.display_name ?? "");
  const [description, setDescription] = useState(agent.description);
  const [persona, setPersona] = useState(agent.persona ?? "");
  return (
    <Card>
      <Label>Shared manager profile</Label>
      <Label muted>
        You can edit the published profile and team memory. Connection
        permissions and agent lifecycle stay with its owner.
      </Label>
      <Field
        label="Shared agent display name"
        value={name}
        onChangeText={setName}
        maxLength={40}
        editable={session.active && !session.busy}
      />
      <Field
        label="Shared agent purpose"
        value={description}
        onChangeText={setDescription}
        maxLength={2048}
        multiline
        editable={session.active && !session.busy}
      />
      <Field
        label="Shared agent persona"
        value={persona}
        onChangeText={setPersona}
        maxLength={2000}
        multiline
        editable={session.active && !session.busy}
      />
      <ActionButton
        label="Save shared profile"
        disabled={!session.active || session.busy || !description.trim()}
        onPress={() =>
          void session.run(
            async () => {
              const fresh = (await agentsApi.detail(agent.id)).agent;
              if (
                !session.valid() ||
                fresh.share_role !== "manager" ||
                fresh.status === "destroyed"
              )
                throw new Error("Manager access changed. Reload the agent.");
              if (
                fresh.description !== baseline.description ||
                fresh.persona !== baseline.persona ||
                fresh.display_name !== baseline.display_name
              )
                throw new Error(
                  "The profile changed elsewhere. Reopen this agent and review the current profile.",
                );
              await agentsApi.update(
                agent.id,
                {
                  display_name: name.trim(),
                  description: description.trim(),
                  persona: persona.trim(),
                },
                { authorityGuard: session.guard },
              );
              return (await agentsApi.detail(agent.id)).agent;
            },
            (result) => {
              setBaseline(result);
              setName(result.display_name ?? "");
              setDescription(result.description);
              setPersona(result.persona ?? "");
            },
          )
        }
      />
    </Card>
  );
}

export function AgentOperationControls({
  agent,
  onOpenChat,
}: {
  agent: AssistantAgent;
  onOpenChat?: () => void;
}) {
  const session = useCollaborationSession();
  const features = session.identity.data?.enabled_features ?? [];
  if (
    !session.readable ||
    agent.kind !== "specialist" ||
    agent.can_maintain !== true ||
    agent.share_role ||
    agent.status === "destroyed"
  )
    return null;
  return (
    <>
      {session.error && <Notice tone="error" message={session.error} />}
      {personalOwner(agent) && (
        <VisitorReadiness
          session={session}
          agent={agent}
          onOpenChat={onOpenChat}
        />
      )}
      {[false, true].map((guests) =>
        features.includes(
          guests
            ? "assistant:guest-operation-scopes"
            : "assistant:operation-scopes",
        ) ? (
          <OperationList
            key={String(guests)}
            session={session}
            agent={agent}
            guests={guests}
          />
        ) : (
          <Label key={String(guests)} muted>
            {guests ? "Guest operation" : "Operation scope"} configuration is
            not enabled. Existing limits still apply.
          </Label>
        ),
      )}
    </>
  );
}

export function OperationList({
  session,
  agent,
  guests,
}: {
  session: Session;
  agent: AssistantAgent;
  guests: boolean;
}) {
  const query = useQuery({
    queryKey: [
      ...session.key,
      agent.id,
      guests ? "guest-operations" : "operations",
    ],
    queryFn: ({ signal }) =>
      collaborationApi.operations(agent.id, guests, signal),
    enabled: session.active,
    retry: false,
  });
  return (
    <Card>
      <Label>
        {guests ? "Guest service operations" : "Agent service operations"}
      </Label>
      <Label muted>
        {guests
          ? "These limits apply to guest turns only. Guest service levels, agent limits and confirmations still apply."
          : "Restrict each granted service to the exact operations this specialist needs."}
      </Label>
      {query.isError && (
        <Notice
          tone="error"
          message="Operations could not load or your permissions changed."
        />
      )}
      {!query.isError &&
        !query.isFetching &&
        query.dataUpdatedAt >= session.activationAt &&
        query.data?.map((service) => (
          <OperationDraft
            key={service.service_id}
            session={session}
            agent={agent}
            service={service}
            guests={guests}
          />
        ))}
      <ActionButton
        label="Refresh operations"
        variant="secondary"
        disabled={session.busy || !session.active}
        onPress={() => void query.refetch()}
      />
    </Card>
  );
}

const stableJson = (value: unknown) =>
  JSON.stringify(value, (_key, entry: unknown) =>
    entry && typeof entry === "object" && !Array.isArray(entry)
      ? Object.fromEntries(
          Object.entries(entry).sort(([a], [b]) => a.localeCompare(b)),
        )
      : entry,
  );

/** Endpoint IDs and explicit rules are sets; object key order is not a change. */
function operationSelectionContent(selection: OperationSelection) {
  return stableJson(
    selection.all_operations
      ? { all_operations: true }
      : {
          all_operations: false,
          endpoint_ids: [...new Set(selection.endpoint_ids)].sort(),
          rules: [...new Set(selection.rules.map(stableJson))].sort(),
          inputs: selection.inputs ?? {},
        },
  );
}

export function OperationDraft({
  session,
  agent,
  service,
  guests,
}: {
  session: Session;
  agent: AssistantAgent;
  service: AgentServiceOperations;
  guests: boolean;
}) {
  const initial = (row: AgentServiceOperations): OperationSelection => ({
    expected_revision: row.revision,
    all_operations: row.all_operations,
    endpoint_ids: row.endpoint_ids,
    rules: row.rules,
  });
  const [value, setValue] = useState(() => initial(service));
  const [search, setSearch] = useState("");
  const stale = value.expected_revision !== service.revision;
  const disabled = session.busy || !session.active;
  const payload = operationPayload(service, value);
  const dirty =
    operationSelectionContent(payload) !==
    operationSelectionContent(operationPayload(service, initial(service)));
  const canSave =
    !disabled &&
    !stale &&
    dirty &&
    operationSelectionSchema.safeParse(value).success;
  return (
    <View style={{ gap: 10 }}>
      <Label>
        {service.service_name} · Revision {service.revision}
      </Label>
      <WorkspaceSelectionField
        label={`${service.service_name} ${guests ? "guest " : ""}operation access`}
        title="Choose operation access"
        value={[value.all_operations ? "all" : "selected"]}
        options={[
          {
            value: "all",
            label: guests ? "All permitted by guest level" : "All operations",
          },
          {
            value: "selected",
            label: "Selected operations (empty blocks all)",
          },
        ]}
        disabled={disabled}
        scopeKey={`${session.epoch}:${agent.id}:${guests}:${service.service_id}:mode`}
        onChange={([mode]) =>
          setValue((old) => ({ ...old, all_operations: mode === "all" }))
        }
      />
      {!value.all_operations && (
        <>
          <Field
            label={`Search ${service.service_name} operations`}
            value={search}
            onChangeText={setSearch}
            editable={!disabled}
          />
          {service.operations.length > 0 ? (
            <>
              <ActionButton
                compact
                label="Select all reads"
                variant="secondary"
                disabled={disabled}
                onPress={() =>
                  setValue((old) => ({
                    ...old,
                    endpoint_ids: service.operations
                      .filter((op) => op.read_only && !op.changes_existing)
                      .map((op) => op.endpoint_id),
                  }))
                }
              />
              <WorkspaceSelectionField
                label="Selected endpoints"
                title="Choose exact operations"
                multiple
                maxSelected={256}
                value={value.endpoint_ids}
                options={service.operations.map((op) => ({
                  value: op.endpoint_id,
                  label: `${op.method} ${op.path}`,
                  description: [
                    op.summary,
                    op.read_only ? "Read only" : "",
                    op.changes_existing ? "Changes existing" : "",
                    service.inputs?.[op.endpoint_id]
                      ? "Saved value limits preserved"
                      : "",
                  ]
                    .filter(Boolean)
                    .join(" · "),
                  searchText: `${op.method} ${op.path} ${op.summary ?? ""}`,
                  disabled:
                    !!search &&
                    !`${op.method} ${op.path} ${op.summary ?? ""}`
                      .toLowerCase()
                      .includes(search.toLowerCase()),
                }))}
                disabled={disabled}
                scopeKey={`${session.epoch}:${agent.id}:${guests}:${service.service_id}:endpoints`}
                onChange={(endpoint_ids) =>
                  setValue((old) => ({ ...old, endpoint_ids }))
                }
              />
            </>
          ) : service.allows_explicit_rules ? (
            <>
              <Label muted>
                Exact methods and paths. A variable such as {"{id}"} matches one
                segment.
              </Label>
              {value.rules.map((rule, index) => (
                <View key={index} style={{ gap: 8 }}>
                  <WorkspaceSelectionField
                    label={`Rule ${index + 1} method`}
                    title="HTTP method"
                    value={[rule.method]}
                    options={[
                      "GET",
                      "HEAD",
                      "OPTIONS",
                      "POST",
                      "PUT",
                      "PATCH",
                      "DELETE",
                    ].map((method) => ({ value: method, label: method }))}
                    disabled={disabled}
                    scopeKey={`${session.epoch}:${agent.id}:${guests}:${service.service_id}:rule:${index}`}
                    onChange={([method]) =>
                      setValue((old) => ({
                        ...old,
                        rules: old.rules.map((r, i) =>
                          i === index
                            ? {
                                ...r,
                                method: z
                                  .enum([
                                    "GET",
                                    "HEAD",
                                    "OPTIONS",
                                    "POST",
                                    "PUT",
                                    "PATCH",
                                    "DELETE",
                                  ])
                                  .parse(method),
                              }
                            : r,
                        ),
                      }))
                    }
                  />
                  <Field
                    label={`Rule ${index + 1} path`}
                    value={rule.path_template}
                    editable={!disabled}
                    onChangeText={(path_template) =>
                      setValue((old) => ({
                        ...old,
                        rules: old.rules.map((r, i) =>
                          i === index ? { ...r, path_template } : r,
                        ),
                      }))
                    }
                  />
                  <ActionButton
                    compact
                    label="Remove rule"
                    variant="secondary"
                    disabled={disabled}
                    onPress={() =>
                      setValue((old) => ({
                        ...old,
                        rules: old.rules.filter((_, i) => i !== index),
                      }))
                    }
                  />
                </View>
              ))}
              <ActionButton
                label="Add method/path rule"
                variant="secondary"
                disabled={disabled || value.rules.length >= 256}
                onPress={() =>
                  setValue((old) => ({
                    ...old,
                    rules: [
                      ...old.rules,
                      { method: "GET", path_template: "/" },
                    ],
                  }))
                }
              />
            </>
          ) : (
            <Label muted>No active catalog operations.</Label>
          )}
          {value.endpoint_ids.some(
            (id) => !service.operations.some((op) => op.endpoint_id === id),
          ) && (
            <ActionButton
              label="Remove unavailable selections"
              variant="secondary"
              disabled={disabled}
              onPress={() =>
                setValue((old) => ({
                  ...old,
                  endpoint_ids: old.endpoint_ids.filter((id) =>
                    service.operations.some((op) => op.endpoint_id === id),
                  ),
                }))
              }
            />
          )}
        </>
      )}
      {stale && (
        <Notice
          message="Operation permissions changed elsewhere. Your draft is kept; reload before saving."
          tone="error"
        />
      )}
      <ActionButton
        label="Reload operation selection"
        variant="secondary"
        disabled={disabled}
        onPress={() => setValue(initial(service))}
      />
      <ActionButton
        label={guests ? "Save guest operations" : "Save operations"}
        disabled={!canSave}
        onPress={() => {
          if (!canSave) return;
          void session.run(
            async () => {
              const fresh = (await agentsApi.detail(agent.id)).agent;
              if (
                !session.valid() ||
                fresh.can_maintain !== true ||
                fresh.share_role ||
                fresh.status === "destroyed"
              )
                throw new Error(
                  "Only a current owner/organization maintainer can edit operation permissions.",
                );
              await collaborationApi.saveOperations(
                agent.id,
                guests,
                service.service_id,
                payload,
                { authorityGuard: session.guard },
              );
            },
            undefined,
            {
              feature: guests
                ? "assistant:guest-operation-scopes"
                : "assistant:operation-scopes",
            },
          );
        }}
      />
    </View>
  );
}

export function VisitorReadiness({
  session,
  agent,
  onOpenChat,
}: {
  session: Session;
  agent: AssistantAgent;
  onOpenChat?: () => void;
}) {
  const query = useQuery({
    queryKey: [...session.key, agent.id, "readiness"],
    queryFn: ({ signal }) => collaborationApi.readiness(agent.id, signal),
    enabled: session.active,
    retry: false,
    refetchInterval: (state) =>
      session.active && state.state.data?.status === "setting_up"
        ? 2000
        : false,
  });
  const readiness =
    !query.isError &&
    !query.isFetching &&
    query.dataUpdatedAt >= session.activationAt
      ? query.data
      : undefined;
  return (
    <Card>
      <Label>Visitor model readiness</Label>
      {query.isError && (
        <Notice
          message="Current visitor readiness could not load."
          tone="error"
        />
      )}
      {!readiness && (
        <Label muted>
          Visitor readiness is unverified. Recheck the current model setup.
        </Label>
      )}
      {readiness && (
        <>
          <Label>
            {readiness.status === "ready"
              ? "Model ready for visitors"
              : readiness.status === "setting_up"
                ? "Setting up for visitors…"
                : "Visitor setup needs attention"}
          </Label>
          <Label muted>
            {readiness.fix ??
              "Service permissions still apply. Visitor usage is billed to you."}
          </Label>
        </>
      )}
      <ActionButton
        label="Recheck visitor readiness"
        variant="secondary"
        disabled={!session.active || session.busy}
        onPress={() => void query.refetch()}
      />
      {readiness?.status === "not_ready" && onOpenChat && (
        <ActionButton
          label="Open agent chat to review visitor setup"
          disabled={!session.active || session.busy}
          onPress={onOpenChat}
        />
      )}
    </Card>
  );
}

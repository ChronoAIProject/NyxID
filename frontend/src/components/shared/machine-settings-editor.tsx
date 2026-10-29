import { useEffect, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { useAppForm } from "@/components/ui/form";
import {
  useNyxBotAgents,
  useSetNyxBotAgentGrants,
} from "@/hooks/use-nyxbot-agents";
import { useAuthStore } from "@/stores/auth-store";
import { useMachineSettingsDraft } from "@/stores/machine-settings-draft-store";
import { NewAgentDialog } from "@/components/assistant/nyxbot-agent-forms";
import { MachineUpdate } from "./machine-update";
import {
  MachineIsolationBadge,
  MachineSharingBadge,
  MachineIsolationDetails,
  MachineSeparationSupport,
} from "./machine-isolation";
import { SettingsSection } from "./machine-settings-section";
import {
  SHARED_LEGACY_WARNING,
  SINGLE_USER_WARNING,
  type MachineProfile,
  type MachineUpdateStatus,
} from "@/schemas/machines";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";

export interface MachinePolicyValues {
  confirm: "none" | "changes" | "all";
  allow: boolean;
  automatic: boolean;
  acknowledged: boolean;
}
export interface MachineSettingsEditorProps {
  readonly machineId: string;
  readonly ownerId?: string;
  readonly name: string;
  readonly machine?: MachineProfile | null;
  readonly connected: boolean;
  readonly desktopReady?: boolean;
  readonly agentVersion?: string | null;
  readonly confirmation?: "none" | "changes" | "all" | null;
  readonly allowSharedLogins?: boolean | null;
  readonly readOnlyPolicy?: boolean;
  readonly editable: boolean;
  readonly canManage?: boolean;
  readonly historical?: boolean;
  readonly separationSupport?: boolean | null;
  readonly updateStatus?: MachineUpdateStatus;
  readonly onSavePolicy?: (values: MachinePolicyValues) => Promise<void>;
  readonly beforeVersion?: ReactNode;
  readonly statusContent?: ReactNode;
  readonly separationStatusContent?: ReactNode;
  readonly membershipStatusContent?: ReactNode;
  readonly afterSettings?: ReactNode;
  readonly onLogins?: () => void;
}
/** Shared presentation and membership adapter. Only the self-hosted caller supplies policy writes. */
export function MachineSettingsEditor(props: MachineSettingsEditorProps) {
  const {
    machineId,
    machine,
    editable,
    readOnlyPolicy = false,
    canManage = true,
  } = props;
  const { draft, update, actor } = useMachineSettingsDraft(
    props.ownerId ?? "personal",
    machineId,
  );
  const agents = useNyxBotAgents(canManage);
  const grants = useSetNyxBotAgentGrants();
  const [creating, setCreating] = useState(false);
  const [removal, setRemoval] = useState(false);
  const [error, setError] = useState<string>();
  const form = useAppForm({
    defaultValues: {
      confirm: draft.confirm ?? props.confirmation ?? "none",
      allow: draft.allow ?? props.allowSharedLogins ?? false,
      automatic: draft.automatic ?? props.updateStatus?.automatic ?? false,
      acknowledged: false,
    },
  });
  const values = {
    confirm: draft.confirm ?? props.confirmation ?? "none",
    allow: draft.allow ?? props.allowSharedLogins ?? false,
    automatic: draft.automatic ?? props.updateStatus?.automatic ?? false,
    acknowledged: form.watch("acknowledged"),
  };
  const options = (agents.data?.agents ?? []).filter(
    (agent) =>
      agent.status !== "destroyed" &&
      (!readOnlyPolicy ||
        (agent.owner_kind !== "org" &&
          (!agent.owner_id || agent.owner_id === actor))),
  );
  const specialists = options.filter((agent) => agent.kind !== "nyxbot");
  const savedMember = (id: string, machines?: string[] | null) =>
    draft.confirmed[id] ?? machines?.includes(machineId) ?? false;
  const selected = (id: string, machines?: string[] | null) =>
    draft.members[id] ?? savedMember(id, machines);
  const changes = specialists.filter(
    (agent) =>
      draft.members[agent.id] !== undefined &&
      draft.members[agent.id] !== savedMember(agent.id, agent.machines),
  );
  const unresolvedMemberDrafts = Object.keys(draft.members).filter(
    (id) => !specialists.some((agent) => agent.id === id),
  );
  const policyChanged =
    !readOnlyPolicy &&
    (values.confirm !== (props.confirmation ?? "none") ||
      values.allow !== (props.allowSharedLogins ?? false) ||
      Boolean(
        props.updateStatus && values.automatic !== props.updateStatus.automatic,
      ));
  const changed = changes.length > 0 || unresolvedMemberDrafts.length > 0 || policyChanged;
  const membershipBlocked =
    agents.isPending || Boolean(agents.error) || !agents.data || unresolvedMemberDrafts.length > 0;
  const disabled = !editable || !canManage || Boolean(draft.saving);
  const risk =
    !readOnlyPolicy &&
    values.allow &&
    !props.allowSharedLogins &&
    !values.acknowledged;
  // Stop masking a saved member once the authoritative read catches up.
  useEffect(() => {
    const caughtUp = options
      .filter(
        (agent) =>
          draft.confirmed[agent.id] !== undefined &&
          draft.confirmed[agent.id] ===
            (agent.machines?.includes(machineId) ?? false),
      )
      .map((agent) => agent.id);
    if (caughtUp.length)
      update((current) => ({
        ...current,
        confirmed: Object.fromEntries(
          Object.entries(current.confirmed).filter(
            ([id]) => !caughtUp.includes(id),
          ),
        ),
      }));
  });
  async function save() {
    if (disabled || risk || !changed || (!policyChanged && membershipBlocked))
      return;
    setError(undefined);
    update((current) => ({ ...current, saving: true, errors: [] }));
    const errors: string[] = [];
    try {
      if (policyChanged && props.onSavePolicy) {
        try {
          await props.onSavePolicy(values);
          update((current) => ({
            ...current,
            confirm: undefined,
            allow: undefined,
            automatic: undefined,
          }));
          form.reset({ ...values, acknowledged: false });
        } catch {
          errors.push(
            "Machine policy could not be saved. Review the current settings and retry.",
          );
        }
      }
      if (membershipBlocked && Object.keys(draft.members).length) {
        errors.push(
          agents.error
            ? "Specialist selections remain unsaved because the Agent list could not be refreshed."
            : agents.isPending || !agents.data
              ? "Specialist selections remain unsaved while the Agent list is loading."
              : "Some Specialist selections remain unsaved because an Agent is no longer in the current list.",
        );
      }
      for (const agent of membershipBlocked ? [] : changes) {
        // A closed drawer may leave a save in flight, but a changed account must stop it.
        if (useAuthStore.getState().user?.id !== actor) break;
        const assigned = draft.members[agent.id]!;
        try {
          await grants.mutateAsync({
            id: agent.id,
            machine_membership: { node_id: machineId, assigned },
          });
          update((current) => ({
            ...current,
            members: Object.fromEntries(
              Object.entries(current.members).filter(([id]) => id !== agent.id),
            ),
            confirmed: { ...current.confirmed, [agent.id]: assigned },
          }));
        } catch {
          errors.push(`${agent.display_name ?? agent.name} needs another try.`);
        }
      }
    } finally {
      update((current) => ({ ...current, saving: false, errors }));
    }
  }
  function discard() {
    update((current) => ({ members: {}, confirmed: current.confirmed }));
    form.reset({
      confirm: props.confirmation ?? "none",
      allow: props.allowSharedLogins ?? false,
      automatic: props.updateStatus?.automatic ?? false,
      acknowledged: false,
    });
    setError(undefined);
    void agents.refetch?.();
  }
  return (
    <>
      <form
        aria-label="Machine settings"
        onSubmit={form.handleSubmit(() => {
          if (
            changes.some(
              (agent) =>
                savedMember(agent.id, agent.machines) &&
                !draft.members[agent.id],
            )
          )
            setRemoval(true);
          else void save();
        })}
        className="flex min-h-0 min-w-0 flex-1 flex-col text-12"
      >
        <div
          aria-label="Machine settings sections"
          className="min-h-0 min-w-0 flex-1 space-y-4 overflow-x-hidden overflow-y-auto px-5 pb-5 [overflow-wrap:anywhere]"
        >
          {props.beforeVersion}
          <SettingsSection
            title={readOnlyPolicy ? "Version" : "Version and updates"}
          >
            {!readOnlyPolicy && props.updateStatus ? (
              <>
                <MachineUpdate name={props.name} status={props.updateStatus} />
                <label className="flex items-start gap-2">
                  <Checkbox
                    checked={values.automatic}
                    disabled={disabled}
                    onCheckedChange={(v) => {
                      form.setValue("automatic", v === true);
                      update((d) => ({ ...d, automatic: v === true }));
                    }}
                  />
                  Update automatically when idle
                </label>
                <p className="text-muted-foreground">
                  Updates wait until no job, turn, live desktop or owner control
                  is active. You receive an update notification.
                </p>
              </>
            ) : (
              <p>Installed: {props.agentVersion ?? "Not reported"}</p>
            )}
          </SettingsSection>
          <SettingsSection title="Status and capabilities" id="machine-status">
            <div className="flex flex-wrap gap-2">
              <Badge variant={props.connected ? "success" : "secondary"}>
                {props.connected ? "Connected" : "Disconnected"}
              </Badge>
              {machine && <MachineIsolationBadge machine={machine} />}
              <MachineSharingBadge />
            </div>
            {props.historical && (
              <p className="text-muted-foreground">
                Last reported information · not a current online report.
              </p>
            )}
            {machine ? (
              <>
                <p>
                  <span role="group" aria-label="Enabled capability badges">
                    {[
                      machine.shell && "Commands",
                      machine.files && "Files",
                      (machine.browser ?? machine.computer) && "Browser",
                      machine.computer && "Computer",
                    ]
                      .filter(Boolean)
                      .map((label, index) => (
                        <span key={String(label)}>
                          {index > 0 && " · "}
                          <span>{label}</span>
                        </span>
                      ))}
                  </span>{" "}
                  · {machine.os} / {machine.arch}
                </p>
                <p className="text-muted-foreground">
                  Workspace roots: {machine.roots.join(", ") || "Not reported"}
                </p>
                <MachineIsolationDetails machine={machine} />
                <p className="text-muted-foreground">
                  Agents configured for Shared legacy can run granted commands
                  and file tools in shared mode, even when separation is
                  unavailable. A separated assignment refuses execution if its
                  separate workspace is unavailable; it never falls back to
                  shared mode.
                </p>
                <MachineSeparationSupport
                  machine={readOnlyPolicy ? undefined : machine}
                  support={
                    props.separationSupport == null
                      ? null
                      : {
                          available: props.separationSupport,
                          reason: props.separationSupport
                            ? null
                            : "separated_unavailable",
                        }
                  }
                />
              </>
            ) : (
              <p className="text-muted-foreground">
                Awaiting machine report. Version, capabilities and separation
                support are not reported.
              </p>
            )}
            {props.statusContent}
          </SettingsSection>
          {machine?.computer && /mac|darwin/i.test(machine.os) && (
            <SettingsSection title="Computer permissions">
              <p>Screen Recording: {machine.computer_permissions?.screen_recording == null ? "Not reported" : machine.computer_permissions.screen_recording ? "Allowed" : "Not allowed"}</p>
              <p>Accessibility: {machine.computer_permissions?.accessibility == null ? "Not reported" : machine.computer_permissions.accessibility ? "Allowed" : "Not allowed"}</p>
              <p className="text-muted-foreground">
                In System Settings, open Privacy &amp; Security and allow Screen Recording and Accessibility for the app that launches the NyxID Node. Restart the Node after changing permissions.
              </p>
            </SettingsSection>
          )}
          {(machine?.computer || machine?.browser) && (
            <SettingsSection title="Live desktop">
              <p className="text-muted-foreground">
                {props.connected &&
                (props.desktopReady ?? machine.computer_ready)
                  ? "Ready"
                  : "Display unavailable"}{" "}
                · {machine.computer_mode ?? "Not reported"} mode · cua{" "}
                {machine.cua_version ?? "unavailable"}
              </p>
              {props.connected &&
              (props.desktopReady ?? machine.computer_ready) ? (
                <Button asChild>
                  <Link
                    to="/assistant/machines/$nodeId/desktop"
                    params={{ nodeId: machineId }}
                  >
                    Open live desktop
                  </Link>
                </Button>
              ) : (
                <Button disabled>Open live desktop</Button>
              )}
            </SettingsSection>
          )}
          {(canManage || readOnlyPolicy) && (
            <SettingsSection title="Owner confirmation">
              <p className="text-muted-foreground">
                {readOnlyPolicy
                  ? "When an agent must ask before acting on this machine."
                  : "Choose when an agent must ask before acting on this machine."}
              </p>
              {readOnlyPolicy && props.confirmation == null ? (
                <p>Not reported</p>
              ) : (
                <Select
                  value={values.confirm}
                  disabled={disabled || readOnlyPolicy}
                  onValueChange={(v) => {
                    form.setValue("confirm", v as typeof values.confirm);
                    update((d) => ({
                      ...d,
                      confirm: v as typeof values.confirm,
                    }));
                  }}
                >
                  <SelectTrigger aria-label="Owner confirmation">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="none">No confirmation</SelectItem>
                    <SelectItem value="changes">Confirm changes</SelectItem>
                    <SelectItem value="all">Confirm every operation</SelectItem>
                  </SelectContent>
                </Select>
              )}
              {readOnlyPolicy && (
                <p className="text-11 text-muted-foreground">
                  Managed for this hosted machine · view only.
                </p>
              )}
            </SettingsSection>
          )}
          {(canManage || readOnlyPolicy) &&
            (machine?.browser ?? machine?.computer) &&
            !machine?.browser_isolated && (
              <SettingsSection title="Saved logins on a shared OS user">
                <p className="text-muted-foreground">{SINGLE_USER_WARNING}</p>
                {readOnlyPolicy && props.allowSharedLogins == null ? (
                  <p>Not reported</p>
                ) : (
                  <label className="flex items-start gap-3">
                    <Checkbox
                      checked={values.allow}
                      disabled={disabled || readOnlyPolicy}
                      onCheckedChange={(v) => {
                        form.setValue("allow", v === true);
                        form.setValue("acknowledged", false);
                        update((d) => ({ ...d, allow: v === true }));
                      }}
                    />
                    Allow saved-login typing on this machine
                  </label>
                )}
                {readOnlyPolicy ? (
                  <p className="text-11 text-muted-foreground">
                    Managed for this hosted machine · view only.
                  </p>
                ) : (
                  values.allow &&
                  !props.allowSharedLogins && (
                    <label className="flex items-start gap-3 rounded-lg border border-warning/30 p-3">
                      <Checkbox
                        checked={values.acknowledged}
                        onCheckedChange={(v) =>
                          form.setValue("acknowledged", v === true)
                        }
                      />
                      I understand the shared-user risk and allow typing.
                    </label>
                  )
                )}
              </SettingsSection>
            )}
          {canManage && (
            <>
              <SettingsSection
                title="Agent separation choices"
                id="machine-separation"
              >
                <p className="text-muted-foreground">
                  New assignments need a recorded choice and confirmed support.
                  Existing Shared legacy assignments retain their configured
                  access. Open an agent’s access settings to select supported
                  separation or explicitly approve shared legacy access through
                  its owner card.
                </p>
                {props.separationStatusContent}
                <p className="text-muted-foreground">{SHARED_LEGACY_WARNING}</p>
                {!editable && (
                  <p role="status">
                    Resume to change access. Existing permissions are kept.
                    Access can be changed when the machine is running.
                  </p>
                )}
                {options
                  .filter(
                    (agent) =>
                      agent.kind === "nyxbot" ||
                      savedMember(agent.id, agent.machines),
                  )
                  .map((agent) => {
                    const label = `Review ${agent.display_name ?? agent.name} separation`;
                    const reviewDisabled = disabled || agent.can_maintain === false ||
                      (agent.kind !== "nyxbot" && !selected(agent.id, agent.machines));
                    return reviewDisabled ? (
                      <Button key={agent.id} type="button" variant="outline" disabled>
                        {label}
                      </Button>
                    ) : (
                      <Button key={agent.id} asChild variant="outline" className="max-w-full whitespace-normal">
                        <Link to="/assistant" search={{ agent: agent.id, details: "grants" }}>
                          {label}
                        </Link>
                      </Button>
                    );
                  })}
              </SettingsSection>
              <SettingsSection title="Specialist access">
                {props.membershipStatusContent}
                <p className="text-muted-foreground">
                  NyxBot uses configured assignments on machines you own or
                  administer. Choose which specialists may use this one, then
                  review their separation choices and capabilities.
                </p>
                <p className="text-muted-foreground">
                  New assignments start with no tools or saved logins.
                  Membership alone does not authorize execution.
                </p>
                {agents.isPending && !agents.data ? (
                  <p role="status">Loading specialists…</p>
                ) : agents.error ? (
                  <p role="alert">Could not load specialists. Existing selections are shown but cannot be changed until the list refreshes.</p>
                ) : null}
                {agents.data && specialists.length ? (
                  <fieldset className="space-y-3" disabled={disabled || membershipBlocked}>
                    <legend className="sr-only">
                      Specialists allowed on this machine
                    </legend>
                    {specialists.map((agent) => (
                      <label
                        key={agent.id}
                        className="flex min-w-0 items-start gap-3"
                      >
                        <Checkbox
                          checked={selected(agent.id, agent.machines)}
                          disabled={agent.can_maintain === false}
                          onCheckedChange={(v) =>
                            update((d) => ({
                              ...d,
                              members: { ...d.members, [agent.id]: v === true },
                            }))
                          }
                        />
                        <span className="min-w-0">
                          {agent.display_name ?? agent.name}
                        </span>
                      </label>
                    ))}
                  </fieldset>
                ) : agents.data && !specialists.length ? (
                  <>
                    <p className="text-muted-foreground">
                      No specialists yet. Create one for a focused role, then
                      grant it this machine.
                    </p>
                    <Button
                      type="button"
                      disabled={disabled}
                      onClick={() => setCreating(true)}
                    >
                      Create a specialist
                    </Button>
                  </>
                ) : null}
                {agents.error && (
                  <Button type="button" onClick={() => void agents.refetch()}>
                    Try again
                  </Button>
                )}
                {agents.isPending && agents.data && (
                  <p role="status">Refreshing specialists… current selections are unchanged.</p>
                )}
                {unresolvedMemberDrafts.length > 0 && (
                  <p role="alert">Some unsaved Specialist selections do not match the current Agent list and remain in this Machine draft.</p>
                )}
                {Boolean(draft.errors?.length) && (
                  <div
                    role="alert"
                    className="space-y-1 rounded-lg border border-warning/30 p-3"
                  >
                    <p>Some selections weren't saved</p>
                    {draft.errors!.map((message) => (
                      <p key={message}>{message}</p>
                    ))}
                    <p>
                      Other changes are saved. Retry the remaining changes
                      below.
                    </p>
                  </div>
                )}
              </SettingsSection>
            </>
          )}
          <SettingsSection title="Saved logins">
            <p className="text-muted-foreground">
              Manage website logins separately. Agents can fill approved fields
              in the secure browser; the developer browser never receives saved
              logins.
            </p>
            {props.onLogins ? (
              <Button type="button" variant="outline" onClick={props.onLogins}>
                Manage saved logins
              </Button>
            ) : (
              <Button asChild variant="outline">
                <Link
                  to="/assistant/machines"
                  search={{ tab: "logins", machine: machineId }}
                >
                  Manage saved logins
                </Link>
              </Button>
            )}
          </SettingsSection>
          {props.afterSettings}
        </div>
        {canManage && (
          <footer className="shrink-0 space-y-2 border-t border-border bg-surface px-5 py-4">
            {changed && (
              <p aria-live="polite" className="text-11 text-muted-foreground">
                {draft.saving
                  ? "Saving Specialist selections…"
                  : "Unsaved machine settings · kept while you review"}
              </p>
            )}
            {error && <p role="alert">{error}</p>}
            <div className="flex items-center justify-end gap-3">
              {changed && (
                <Button
                  type="button"
                  variant="ghost"
                  disabled={draft.saving}
                  onClick={discard}
                >
                  Discard changes
                </Button>
              )}
              <Button
                type="submit"
                variant="primary"
                isLoading={draft.saving}
                disabled={
                  disabled ||
                  !changed ||
                  risk ||
                  !policyChanged && membershipBlocked
                }
              >
                Save machine settings
              </Button>
            </div>
          </footer>
        )}
      </form>
      {creating && (
        <NewAgentDialog
          machineAssignment="membership_only"
          onClose={() => setCreating(false)}
          onCreated={() => setCreating(false)}
        />
      )}
      {removal && (
        <Dialog open onOpenChange={setRemoval}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Remove Specialist access?</DialogTitle>
              <DialogDescription>
                The removed Specialists lose this machine’s tool and login
                permissions. Your workspace, account saved logins and other
                assistants’ permissions are kept.
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => setRemoval(false)}
              >
                Cancel
              </Button>
              <Button
                type="button"
                variant="destructive"
                onClick={() => {
                  setRemoval(false);
                  void save();
                }}
              >
                Save and remove access
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </>
  );
}

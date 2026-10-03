import {
  MachineIsolationBadge,
  MachineSharingBadge,
  MachineIsolationDetails,
} from "@/components/shared/machine-isolation";
import { useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { useAppForm } from "@/components/ui/form";
import { MachineUpdate } from "@/components/shared/machine-update";
import type { MachineUpdateStatus } from "@/schemas/machines";
import {
  useMachineUpdatePolicy,
  useMachineSettings,
} from "@/hooks/use-machines";
import {
  useNyxBotAgents,
  useSetNyxBotAgentGrants,
} from "@/hooks/use-nyxbot-agents";
import { NewAgentDialog } from "@/components/assistant/nyxbot-agent-forms";
import { SINGLE_USER_WARNING } from "@/schemas/machines";
import type { NodeInfo } from "@/types/nodes";
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

function SettingsSection({
  title,
  children,
}: {
  readonly title: string;
  readonly children: ReactNode;
}) {
  return (
    <section className="min-w-0 space-y-3 rounded-xl border border-border/50 bg-card p-4">
      <h3 className="text-[13px] font-semibold text-foreground">{title}</h3>
      {children}
    </section>
  );
}

export function MachineSettings({
  node,
  canManage,
  update,
}: {
  readonly node: NodeInfo;
  readonly canManage: boolean;
  readonly update?: MachineUpdateStatus;
}) {
  const settings = useMachineSettings(node.id);
  const updatePolicy = useMachineUpdatePolicy(node.id);
  const [automaticDraft, setAutomaticDraft] = useState<boolean>();
  const automatic = automaticDraft ?? update?.automatic ?? false;
  const agents = useNyxBotAgents();
  const grants = useSetNyxBotAgentGrants();
  const form = useAppForm({
    defaultValues: {
      confirm: node.machine_confirm ?? "none",
      allow: node.allow_single_user_saved_logins ?? false,
    },
  });
  const confirm = form.watch("confirm"),
    allow = form.watch("allow");
  const [acknowledged, setAcknowledged] = useState(false);
  const [draftGrants, setDraftGrants] = useState<Record<string, boolean>>({});
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string>();
  const [saving, setSaving] = useState(false);
  const machine = node.machine;
  const specialists =
    agents.data?.agents.filter(
      (agent) => agent.kind !== "nyxbot" && agent.status !== "destroyed",
    ) ?? [];
  const grantChanges = specialists.filter(
    (agent) =>
      draftGrants[agent.id] !== undefined &&
      draftGrants[agent.id] !== (agent.machines?.includes(node.id) ?? false),
  );
  const settingsChanged = form.formState.isDirty;
  const automaticChanged = update && automatic !== update.automatic;
  const changed =
    settingsChanged || grantChanges.length > 0 || automaticChanged;
  async function save() {
    setError(undefined);
    setSaving(true);
    try {
      if (automaticChanged) {
        await updatePolicy.mutateAsync(automatic);
        setAutomaticDraft(undefined);
      }
      if (settingsChanged) {
        await settings.mutateAsync({
          machine_confirm: confirm,
          allow_single_user_saved_logins: allow,
          acknowledge_single_user_risk: acknowledged,
        });
        form.reset({ confirm, allow });
      }
      for (const agent of grantChanges) {
        await grants.mutateAsync({
          id: agent.id,
          services: agent.services,
          account_read: agent.account_read,
          machines: draftGrants[agent.id]
            ? [...new Set([...(agent.machines ?? []), node.id])]
            : (agent.machines ?? []).filter((id) => id !== node.id),
        });
        setDraftGrants((previous) => {
          const next = { ...previous };
          delete next[agent.id];
          return next;
        });
      }
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not save machine settings.",
      );
    } finally {
      setSaving(false);
    }
  }
  if (!machine) return null;
  return (
    <>
      <form
        aria-label="Machine settings"
        onSubmit={form.handleSubmit(save)}
        className="flex min-h-0 min-w-0 flex-1 flex-col text-[12px]"
      >
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 pb-5 [overflow-wrap:anywhere]">
          {update ? (
            <SettingsSection title="Version and updates">
              <MachineUpdate name={node.name} status={update} />
              <label className="flex items-start gap-2">
                <Checkbox
                  checked={automatic}
                  disabled={!canManage}
                  onCheckedChange={(value) => setAutomaticDraft(value === true)}
                />
                <span>
                  Update automatically when idle
                  <span className="mt-1 block text-muted-foreground">
                    Recommended. Updates wait until no job, turn, live desktop
                    or owner control is active. You receive an update
                    notification.
                  </span>
                </span>
              </label>
            </SettingsSection>
          ) : null}
          <SettingsSection title="Status and capabilities">
            <div className="flex flex-wrap gap-2">
              <Badge variant={node.is_connected ? "success" : "secondary"}>
                {node.is_connected ? "Connected" : "Disconnected"}
              </Badge>
              <MachineIsolationBadge machine={machine} />
              <MachineSharingBadge />
            </div>
            <p>
              {[
                machine.shell && "Commands",
                machine.files && "Files",
                machine.computer && "Computer",
              ]
                .filter(Boolean)
                .join(" · ")}{" "}
              · {machine.os} / {machine.arch}
            </p>
            <p className="text-muted-foreground">
              Workspace roots: {machine.roots.join(", ")}
            </p>
            <MachineIsolationDetails machine={machine} />
          </SettingsSection>
          {machine.computer ? (
            <SettingsSection title="Live desktop">
              <p className="text-muted-foreground">
                {machine.computer_ready
                  ? "Ready"
                  : "Permissions or display unavailable"}{" "}
                · {machine.computer_mode} mode · cua{" "}
                {machine.cua_version ?? "unavailable"}
              </p>
              {machine.computer_permissions ? (
                <p className="text-muted-foreground">
                  Screen Recording:{" "}
                  {machine.computer_permissions.screen_recording
                    ? "allowed"
                    : "not granted"}
                  . Accessibility:{" "}
                  {machine.computer_permissions.accessibility
                    ? "allowed"
                    : "not granted"}
                  . Enable permissions for the app running the node in macOS
                  System Settings.
                </p>
              ) : null}
              <Button asChild className="max-w-full">
                <Link
                  to="/assistant/machines/$nodeId/desktop"
                  params={{ nodeId: node.id }}
                  search={{ conversation_id: undefined }}
                >
                  Open live desktop
                </Link>
              </Button>
            </SettingsSection>
          ) : null}
          {canManage ? (
            <SettingsSection title="Owner confirmation">
              <p className="text-muted-foreground">
                Choose when an agent must ask before acting on this machine.
              </p>
              <Select
                value={confirm}
                onValueChange={(value) =>
                  form.setValue("confirm", value as typeof confirm)
                }
                disabled={saving}
              >
                <SelectTrigger
                  aria-label="Owner confirmation"
                  className="w-full"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">No confirmation</SelectItem>
                  <SelectItem value="changes">Confirm changes</SelectItem>
                  <SelectItem value="all">Confirm every operation</SelectItem>
                </SelectContent>
              </Select>
            </SettingsSection>
          ) : null}
          {canManage && machine.computer && !machine.browser_isolated ? (
            <SettingsSection title="Saved logins on a shared OS user">
              <p className="text-muted-foreground">{SINGLE_USER_WARNING}</p>
              <label className="flex items-start gap-3">
                <Checkbox
                  className="mt-0.5 shrink-0"
                  checked={allow}
                  disabled={saving}
                  onCheckedChange={(value) => {
                    form.setValue("allow", value === true);
                    setAcknowledged(false);
                  }}
                />
                <span>Allow saved-login typing on this machine</span>
              </label>
              {allow && !node.allow_single_user_saved_logins ? (
                <label className="flex items-start gap-3 rounded-lg border border-warning/30 p-3">
                  <Checkbox
                    className="mt-0.5 shrink-0"
                    checked={acknowledged}
                    onCheckedChange={(value) => setAcknowledged(value === true)}
                  />
                  <span>
                    I understand the shared-user risk and allow typing.
                  </span>
                </label>
              ) : null}
            </SettingsSection>
          ) : null}
          {canManage ? (
            <SettingsSection title="Specialist access">
              <p className="text-muted-foreground">
                NyxBot can use every machine you own or administer. Choose which
                specialists can use this one.
              </p>
              {agents.isPending ? (
                <p role="status">Loading specialists…</p>
              ) : agents.error ? (
                <p role="alert" className="text-destructive">
                  {agents.error.message}
                </p>
              ) : specialists.length ? (
                <fieldset disabled={saving} className="space-y-3">
                  <legend className="sr-only">
                    Specialists allowed on this machine
                  </legend>
                  {specialists.map((agent) => (
                    <label
                      key={agent.id}
                      className="flex min-w-0 items-start gap-3"
                    >
                      <Checkbox
                        className="mt-0.5 shrink-0"
                        checked={
                          draftGrants[agent.id] ??
                          agent.machines?.includes(node.id) ??
                          false
                        }
                        onCheckedChange={(checked) =>
                          setDraftGrants((previous) => ({
                            ...previous,
                            [agent.id]: checked === true,
                          }))
                        }
                      />
                      <span className="min-w-0">
                        {agent.display_name ?? agent.name}
                      </span>
                    </label>
                  ))}
                </fieldset>
              ) : (
                <div className="space-y-3">
                  <p className="text-muted-foreground">
                    No specialists yet. Create one for a focused role, then
                    grant it this machine.
                  </p>
                  <Button type="button" onClick={() => setCreating(true)}>
                    Create a specialist
                  </Button>
                </div>
              )}
            </SettingsSection>
          ) : null}
          <SettingsSection title="Saved logins">
            <p className="text-muted-foreground">
              Manage website logins separately. Agents can fill approved fields
              in the secure browser; the developer browser never receives saved
              logins.
            </p>
            <Button
              asChild
              variant="outline"
              className="max-w-full whitespace-normal"
            >
              <Link to="/assistant/machines" search={{ tab: "logins" }}>
                Manage saved logins
              </Link>
            </Button>
          </SettingsSection>
        </div>
        {canManage ? (
          <footer className="shrink-0 space-y-2 border-t border-border bg-surface px-5 py-4">
            {error ? (
              <p role="alert" className="break-words text-destructive">
                {error}
              </p>
            ) : null}
            <div className="flex items-center justify-end gap-3">
              <Button
                type="submit"
                variant="primary"
                disabled={
                  !changed ||
                  (allow &&
                    !node.allow_single_user_saved_logins &&
                    !acknowledged)
                }
                isLoading={saving}
              >
                Save machine settings
              </Button>
            </div>
          </footer>
        ) : null}
      </form>
      {creating ? (
        <NewAgentDialog
          onClose={() => setCreating(false)}
          onCreated={() => setCreating(false)}
        />
      ) : null}
    </>
  );
}

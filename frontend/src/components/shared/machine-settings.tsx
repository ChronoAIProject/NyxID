import { Label } from "@/components/ui/label";
import { CompactScroll } from "@/components/ui/compact-title";
import { FormViewport } from "@/components/ui/form-viewport";
import { describeChanges, labelValue } from "@/lib/grouped-form-changes";
import { zodResolver } from "@hookform/resolvers/zod";
import { machineSettingsFormSchema } from "@/schemas/machines";
import {
  GroupedList,
  GroupedRow,
  GroupedSelectTrigger as SelectTrigger,
} from "@/components/ui/grouped-list";
import { GroupedFormActions } from "@/components/ui/grouped-form";
import { ChevronRight } from "lucide-react";
import {
  MachineIsolationDetails,
  MachineSeparationSupport,
} from "@/components/shared/machine-isolation";
import { useState } from "react";
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
import { SHARED_LEGACY_WARNING, SINGLE_USER_WARNING } from "@/schemas/machines";
import type { NodeInfo } from "@/types/nodes";
import { Checkbox } from "@/components/ui/checkbox";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";

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
    resolver: zodResolver(machineSettingsFormSchema),
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
  function discard() {
    form.reset();
    setAutomaticDraft(undefined);
    setDraftGrants({});
    setAcknowledged(false);
    setError(undefined);
  }
  if (!machine) return null;
  return (
    <>
      <form
        aria-label="Machine settings"
        onSubmit={form.handleSubmit(save)}
        className="flex min-h-0 min-w-0 flex-1 flex-col text-14"
      >
        <FormViewport className="flex min-h-0 flex-1 flex-col"><CompactScroll className="flex-1 space-y-6 px-5 pb-5 [overflow-wrap:anywhere]">
          {update && (
            <>
              <MachineUpdate grouped name={node.name} status={update} />
              <GroupedList
                title="Automatic updates"
                footer="Recommended. Updates wait until no job, turn, live desktop or owner control is active. You receive an update notification."
              >
                <GroupedRow asChild interactive>
                  <Label>
                    <span className="min-w-0 flex-1">
                      Update automatically when idle
                    </span>
                    <Switch
                      checked={automatic}
                      disabled={!canManage}
                      onCheckedChange={(value) => setAutomaticDraft(value === true)}
                    />
                  </Label>
                </GroupedRow>
              </GroupedList>
            </>
          )}
          <GroupedList
            title="Status and capabilities"
            footer={
              <>
                <p>Workspace roots: {machine.roots.join(", ")}</p>
                <MachineIsolationDetails machine={machine} />
                <p>
                  Agents configured for Shared legacy can run granted commands
                  and file tools in shared mode, even when separation is
                  unavailable. A separated assignment refuses execution if its
                  separate workspace is unavailable; it never falls back to
                  shared mode.
                </p>
                <MachineSeparationSupport humanOnly machine={machine} />
              </>
            }
          >
            <GroupedRow
              title={node.is_connected ? "Connected" : "Disconnected"}
              description={[machine.os, machine.arch].join(" / ")}
            />
            <GroupedRow
              title="Enabled capabilities"
              description={[
                machine.shell && "Commands",
                machine.files && "Files",
                (machine.browser ?? machine.computer) && "Browser",
                machine.computer && "Computer",
              ]
                .filter(Boolean)
                .join(" · ")}
            />
            <GroupedRow
              title="Sharing set per agent"
              description={
                !machine.shell || machine.commands_isolated === true
                  ? undefined
                  : machine.commands_isolated === false
                    ? "Not isolated"
                    : "Isolation unknown: update to check"
              }
            />
          </GroupedList>
          {(machine.computer || machine.browser) && (
            <GroupedList
              title="Live desktop"
              footer={
                machine.computer_permissions && (
                  <p>
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
                )
              }
            >
              <GroupedRow
                title={
                  machine.computer_ready
                    ? "Ready"
                    : "Permissions or display unavailable"
                }
                description={`${machine.computer_mode} mode · cua ${machine.cua_version ?? "unavailable"}`}
              />
              <GroupedRow asChild interactive>
                <Link
                  to="/assistant/machines/$nodeId/desktop"
                  params={{ nodeId: node.id }}
                  search={{ conversation_id: undefined }}
                >
                  <span className="flex-1">Open live desktop</span>
                  <ChevronRight
                    aria-hidden
                    className="size-[1.125em] text-text-tertiary"
                  />
                </Link>
              </GroupedRow>
            </GroupedList>
          )}
          {canManage && (
            <GroupedList
              title="Owner confirmation"
              footer="Choose when an agent must ask before acting on this machine."
            >
              <GroupedRow
                title="Owner confirmation"
                accessory={
                  <Select
                    value={confirm}
                    onValueChange={(value) =>
                      form.setValue("confirm", value as typeof confirm)
                    }
                    disabled={saving}
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
                }
              />
            </GroupedList>
          )}
          {canManage &&
            (machine.browser ?? machine.computer) &&
            !machine.browser_isolated && (
              <GroupedList
                title="Saved logins on a shared OS user"
                footer={<p className="text-warning">{SINGLE_USER_WARNING}</p>}
              >
                <GroupedRow asChild interactive>
                  <Label>
                    <span className="min-w-0 flex-1">
                      Allow saved-login typing on this machine
                    </span>
                    <Switch
                      checked={allow}
                      disabled={saving}
                      onCheckedChange={(value) => {
                        form.setValue("allow", value === true);
                        setAcknowledged(false);
                      }}
                    />
                  </Label>
                </GroupedRow>
                {allow && !node.allow_single_user_saved_logins && (
                  <GroupedRow asChild interactive>
                    <Label>
                      <span className="min-w-0 flex-1">
                        I understand the shared-user risk and allow typing.
                      </span>
                      <Switch
                        checked={acknowledged}
                        disabled={saving}
                        onCheckedChange={(value) => setAcknowledged(value === true)
                        }
                      />
                    </Label>
                  </GroupedRow>
                )}
              </GroupedList>
            )}
          {canManage && (
            <>
              <GroupedList
                title="Agent separation choices"
                footer={
                  <>
                    <p>
                      New assignments need a recorded choice and confirmed
                      support. Existing Shared legacy assignments retain their
                      configured access. Open an agent’s access settings to
                      select supported separation or explicitly approve shared
                      legacy access through its owner card.
                    </p>
                    <p className="text-warning">{SHARED_LEGACY_WARNING}</p>
                  </>
                }
              >
                {(agents.data?.agents ?? [])
                  .filter(
                    agent => agent.status !== "destroyed" && (agent.kind === "nyxbot" || agent.machines?.includes(node.id)),
                  )
                  .map((agent) => (
                    <GroupedRow key={agent.id} asChild interactive>
                      <Link
                        to="/assistant"
                        search={{ agent: agent.id, details: "grants" }}
                      >
                        <span className="min-w-0 flex-1">
                          Review {agent.display_name ?? agent.name} separation
                        </span>
                        <ChevronRight
                          aria-hidden
                          className="size-[1.125em] shrink-0 text-text-tertiary"
                        />
                      </Link>
                    </GroupedRow>
                  ))}
              </GroupedList>
              <GroupedList
                title="Specialist access"
                footer={
                  <>
                    <p>
                      NyxBot uses configured assignments on machines you own or
                      administer. Choose which specialists may use this one,
                      then review their separation choices and capabilities.
                    </p>
                    {agents.error && (
                      <p role="alert" className="text-destructive">
                        {agents.error.message}
                      </p>
                    )}
                  </>
                }
              >
                {agents.isPending ? (
                  <GroupedRow role="status" title="Loading specialists…" />
                ) : specialists.length ? (
                  <fieldset disabled={saving}>
                    <Label asChild><legend className="sr-only">
                      Specialists allowed on this machine
                    </legend></Label>
                    {specialists.map((agent) => (
                      <GroupedRow key={agent.id} asChild interactive>
                        <Label>
                          <span className="min-w-0 flex-1">
                            {agent.display_name ?? agent.name}
                          </span>
                          <Checkbox
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
                        </Label>
                      </GroupedRow>
                    ))}
                  </fieldset>
                ) : (
                  !agents.error && (
                    <>
                      <GroupedRow
                        title="No specialists yet"
                        description="Create one for a focused role, then grant it this machine."
                      />
                      <GroupedRow asChild interactive>
                        <button type="button" onClick={() => setCreating(true)}>
                          Create a specialist
                        </button>
                      </GroupedRow>
                    </>
                  )
                )}
              </GroupedList>
            </>
          )}
          <GroupedList
            title="Saved logins"
            footer={
              <>
                <p>
                  Manage website logins separately. Agents can fill approved
                  fields in the secure browser; the developer browser never
                  receives saved logins.
                </p>
                {error && (
                  <p role="alert" className="text-destructive">
                    {error}
                  </p>
                )}
              </>
            }
          >
            <GroupedRow asChild interactive>
              <Link to="/assistant/machines" search={{ tab: "logins" }}>
                <span className="flex-1">Manage saved logins</span>
                <ChevronRight
                  aria-hidden
                  className="size-[1.125em] text-text-tertiary"
                />
              </Link>
            </GroupedRow>
          </GroupedList>
        </CompactScroll></FormViewport>
        {canManage && (
          <GroupedFormActions
            changes={() => [
              ...describeChanges(form.formState.defaultValues, form.getValues(), [
                { key: "confirm", label: "Confirmations", format: labelValue({ none: "No confirmation", changes: "Confirm changes", all: "Confirm every operation" }) },
                { key: "allow", label: "Allow saved logins on single-user machines" },
              ]),
              ...describeChanges({ automatic: update?.automatic ?? false }, { automatic }, [{ key: "automatic", label: "Update automatically when idle" }]),
              ...grantChanges.map((agent) => ({ label: `Specialist access · ${agent.display_name || agent.name}`, before: agent.machines?.includes(node.id) ? "On" : "Off", after: draftGrants[agent.id] ? "On" : "Off" })),
            ]}
            label="machine settings"
            dirty={Boolean(changed)}
            pending={saving}
            disabled={
              allow && !node.allow_single_user_saved_logins && !acknowledged
            }
            save={form.handleSubmit(save)}
            discard={discard}
            saveLabel="Save changes"
          />
        )}
      </form>
      {creating && (
        <NewAgentDialog
          onClose={() => setCreating(false)}
          onCreated={() => setCreating(false)}
        />
      )}
    </>
  );
}

import { describeChanges, changeList, recordChanges } from "@/lib/grouped-form-changes";
import { useDrawerSearchEntries } from "@/lib/drawer-search";
import {
  GroupedList,
  GroupedRow,
  GroupedDisclosure,
} from "@/components/ui/grouped-list";
import { SHARED_LEGACY_WARNING } from "@/schemas/machines";
import { MachineSeparationSupport } from "@/components/shared/machine-isolation";
import type { MachineContextSelection } from "@/schemas/machine-access";
import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import { Checkbox } from "@/components/ui/checkbox";
import { Switch } from "@/components/ui/switch";
import { GroupedFormActions } from "@/components/ui/grouped-form";
import { GroupedDetail } from "@/components/ui/grouped-detail";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import {
  useMachineAccess,
  useRequestMachineContext,
  useSetMachineAccess,
} from "@/hooks/use-machine-access";
import { machineSelectionSchema, type MachineAccess, type MachineSelection } from "@/schemas/machine-access";
const labels = { shell: "Shell commands", files: "File tools", browser: "Secure browser", computer: "Full computer control", developer_browser: "Developer browser" } as const;
const descriptions: Record<keyof typeof labels, string> = {
  shell: "Run commands as the machine’s OS user",
  files: "Read and write files",
  browser: "Saved logins stay here; browser-only access avoids desktop authority",
  computer: "Operate applications through their UI",
  developer_browser: "Run JavaScript in a separate browser",
};

export function MachineCapabilities({
  agentId,
  disabled = false,
}: {
  readonly agentId: string;
  readonly disabled?: boolean;
}) {
  const enabled = useFeature(FEATURE_FLAG.MACHINE_CAPABILITIES);
  const contextsEnabled = useFeature(FEATURE_FLAG.MACHINE_CONTEXTS);
  const query = useMachineAccess(agentId, !disabled);
  if (!enabled && !contextsEnabled && !query.data?.length) return null;
  return (
    <section aria-label="Machine capabilities" className="space-y-6">
      <GroupedList
        title="Machine access"
        footer={
          <>
            <p>
              Choose what this agent can do on each machine. With capability
              editing enabled, newly selected machines start with all
              permissions off. Existing legacy access stays until edited. Saving
              changes cancels this agent’s current work on that machine.
            </p>
            {!enabled ? (
              <p role="status">
                Capability editing is not enabled yet. Existing restrictions
                still apply.
              </p>
            ) : null}
            {query.error ? (
              <p role="alert" className="text-destructive">
                Could not refresh machine capabilities. Displayed status may be
                out of date.
              </p>
            ) : null}
            {query.data?.some(
              (machine) =>
                machine.revocation_pending || machine.delivery_pending,
            ) ? (
              <p>
                Status refreshes for up to one minute while this page is active.
                Use Refresh machine status for a later check.
              </p>
            ) : null}
          </>
        }
      >
        {query.data?.map((machine) => (
          <MachineCapabilityPage
            key={machine.node_id}
            machine={machine}
            agentId={agentId}
            disabled={disabled}
            capabilityEditing={enabled}
            contextEnabled={contextsEnabled}
          />
        ))}
        {!query.data?.length ? (
          <GroupedRow
            title={
              query.isPending
                ? "Loading machine access…"
                : "No machines assigned"
            }
          />
        ) : null}
        <GroupedRow asChild interactive>
          <button
            type="button"
            disabled={query.isFetching || disabled}
            onClick={() => void query.refetch()}
          >
            Refresh machine status
          </button>
        </GroupedRow>
      </GroupedList>
    </section>
  );
}

function MachineCapabilityPage(
  props: React.ComponentProps<typeof MachineCapabilityForm>,
) {
  const [open, setOpen] = useState(false);
  const machine = props.machine;
  const needsSelection = machine.selection_required === true || machine.mode === "selection_required";
  const searchId = `machine-detail-${machine.node_id}`;
  useDrawerSearchEntries([{ id: searchId, title: machine.name, breadcrumb: "Access › Machine access",
    keywords: `${needsSelection ? "Needs owner selection" : machine.mode === "separated" ? "Separate workspace and browser" : "Shared legacy access"} ${Object.keys(labels).map((key) => labels[key as keyof typeof labels]).join(" ")}`,
    pages: [searchId], reveal: () => setOpen(true),
  }]);
  return (
    <>
      <GroupedDisclosure data-drawer-page-trigger={searchId} onClick={() => setOpen(true)}>
        <span className="block">{machine.name}</span>
        <span className="block text-11 font-normal text-text-tertiary">
          {needsSelection
            ? "Needs owner selection"
            : machine.mode === "separated"
              ? "Separate workspace and browser"
              : "Shared legacy access"}
        </span>
      </GroupedDisclosure>
      <GroupedDetail
        searchPage={searchId}
        open={open}
        title={machine.name}
        info="Review this agent’s workspace separation and allowed tools on this machine. Machine support and permissions still apply."
        onReveal={() => setOpen(true)}
        backLabel="Back to Access"
        onBack={() => setOpen(false)}
      >
        <MachineCapabilityForm key={machine.revision} {...props} />
      </GroupedDetail>
    </>
  );
}

function humanReason(value: string | null | undefined, fallback: string) {
  if (!value) return fallback;
  const text = value.replace(/^[a-z][a-z0-9_]+:\s*/, "");
  return /^[a-z0-9_]+$/.test(text) ? fallback : text;
}

export function MachineCapabilityForm({
  machine,
  agentId,
  disabled = false,
  capabilityEditing = true,
  contextEnabled = false,
  onSaveError,
  onSaveSuccess,
}: {
  readonly machine: MachineAccess;
  readonly agentId: string;
  readonly disabled?: boolean;
  readonly capabilityEditing?: boolean;
  readonly contextEnabled?: boolean;
  readonly onSaveError?: () => void;
  readonly onSaveSuccess?: () => void;
}) {
  const mutation = useSetMachineAccess(agentId);
  const contextMutation = useRequestMachineContext(agentId);
  const [error, setError] = useState<string>();
  const [contextRequested, setContextRequested] = useState(false);
  const [legacyAccepted, setLegacyAccepted] = useState(false);
  const needsSelection = machine.selection_required === true || machine.mode === "selection_required";
  const separationAvailable = machine.separated?.available === true && !machine.separated.reason;
  const form = useAppForm<MachineSelection>({ resolver: zodResolver(machineSelectionSchema), defaultValues: {
    expected_revision: machine.revision, capabilities: machine.capabilities, saved_login_ids: machine.saved_login_ids,
  } });
  const caps = form.watch("capabilities");
  const requestContext = async (mode: MachineContextSelection["mode"] = "separated") => {
    setError(undefined);
    try {
      await contextMutation.mutateAsync({
        node: machine.node_id,
        selection: {
          mode,
          expected_revision: machine.revision,
          capabilities: machine.capabilities,
          saved_login_ids: machine.saved_login_ids ?? [],
        },
      });
      setContextRequested(true);
    } catch {
      setError("Could not request this machine selection. Reload if access changed; automatic and required need current confirmed support.");
    }
  };
  const save = form.handleSubmit(async (selection) => {
    setError(undefined);
    try { await mutation.mutateAsync({ node: machine.node_id, selection }); form.reset(selection); onSaveSuccess?.(); }
    catch { setError("Could not save capabilities. Reload if access changed."); onSaveError?.(); }
  });
  const blocked =
    disabled ||
    !capabilityEditing ||
    needsSelection ||
    !machine.can_edit ||
    !machine.protocol_v2;
  return (
    <form
      className="space-y-6"
      aria-label={`${machine.name} capabilities`}
      onSubmit={save}
    >
      <GroupedList
        title="Workspace and browser"
        footer={
          <>
            {needsSelection ? (
              <p role="status" data-selection-reason={machine.selection_reason}>
                {humanReason(
                  machine.execution_note ?? machine.selection_reason,
                  "Execution is blocked until the owner chooses a supported separation mode or explicitly approves shared legacy access. Open Assistant → Machines to review and approve the selection.",
                )}
              </p>
            ) : (
              <>
                <p>
                  {machine.mode === "separated"
                    ? "This agent has its own workspace and browser profiles. Use a separate machine container or VM per agent for stronger separation; containers still share their host kernel."
                    : "Workspace and browser sessions are shared with other agents in shared legacy mode on this machine."}
                </p>
                <p role="status">
                  {humanReason(
                    machine.execution_note,
                    machine.mode === "separated"
                      ? separationAvailable
                        ? "Granted commands and file tools use this agent’s separated workspace. No shared fallback."
                        : "This assignment requires separated mode, but separation is unavailable. Execution is refused. Restore support or ask the owner to change mode through an owner action card; no automatic shared fallback."
                      : "Granted commands and file tools run in shared mode, as configured, even when separation is unavailable. Shell uses the command user’s full OS permissions.",
                  )}
                </p>
              </>
            )}
            {machine.can_edit && machine.mode !== "shared_legacy" ? (
              <p className="text-warning">{SHARED_LEGACY_WARNING}</p>
            ) : null}
            {contextRequested || contextMutation.isSuccess ? (
              <p role="status">
                Owner approval requested in the NyxBot Assistant thread.
              </p>
            ) : null}
            <MachineSeparationSupport
              support={machine.separated}
              setupNote={machine.separated_setup_note}
              humanOnly
            />
            {machine.quarantine_pending ? (
              <p role="status" className="text-warning">
                Browser cleanup is pending. Reconnect the machine and refresh
                status. New browser work stays blocked until cleanup finishes.
              </p>
            ) : null}
            {machine.revocation_initialization_pending ? (
              <p role="status">
                Checking when earlier commands will stop. Refresh status in a
                few seconds.
              </p>
            ) : (machine.execution_revocation_pending ??
              machine.revocation_pending) ? (
              <p role="status">
                Earlier commands are stopping.
                {machine.revocation_expires_at
                  ? ` They should stop by ${new Date(machine.revocation_expires_at).toLocaleString()}.`
                  : " Reconnect and refresh status to confirm they have stopped."}{" "}
                Refresh status before starting new work.
              </p>
            ) : machine.delivery_pending ? (
              <p role="status">
                Earlier commands have stopped. Updated permissions still need
                machine acknowledgement. Reconnect the machine and refresh
                status.
              </p>
            ) : null}
            {machine.revocation_diagnostic ? (
              <p role="status" className="text-warning">
                {machine.revocation_diagnostic ===
                "authority_timing_upgrade_required"
                  ? "Update and reconnect this machine to confirm permission changes."
                  : "Recovery status needs administrator attention. Earlier commands are still marked as stopping; check machine diagnostics before continuing."}
              </p>
            ) : null}
            {!machine.can_edit ? (
              <p role="status">
                Only the machine owner or an organization admin can edit this
                access.
              </p>
            ) : null}
            {!machine.protocol_v2 ? (
              <p role="status">
                Update this machine before changing capabilities. Its existing
                access continues.
              </p>
            ) : null}
            {error ? (
              <p role="alert" className="text-destructive">
                {error}
              </p>
            ) : null}
          </>
        }
      >
        <GroupedRow
          data-drawer-search-target={`machine-detail-${machine.node_id}`}
          title="Workspace"
          accessory={
            needsSelection
              ? "Needs owner selection"
              : machine.mode === "separated"
                ? "Separate workspace and browser"
                : "Shared legacy"
          }
        />
        {machine.can_edit && machine.mode !== "shared_legacy" ? (
          <>
            <GroupedRow asChild interactive>
              <label>
                <span className="min-w-0 flex-1">
                  I explicitly choose shared legacy access and acknowledge this
                  warning.
                </span>
                <Switch
                  disabled={disabled || contextMutation.isPending}
                  checked={legacyAccepted}
                  onCheckedChange={(value) => setLegacyAccepted(value === true)}
                />
              </label>
            </GroupedRow>
            <GroupedRow asChild interactive>
              <button
                type="button"
                disabled={
                  disabled || !legacyAccepted || contextMutation.isPending
                }
                onClick={() => void requestContext("shared_legacy")}
              >
                Request shared legacy owner approval
              </button>
            </GroupedRow>
          </>
        ) : null}
        {contextEnabled &&
        machine.mode !== "separated" &&
        machine.can_edit &&
        separationAvailable ? (
          <GroupedRow asChild interactive>
            <button
              type="button"
              disabled={disabled || contextMutation.isPending}
              onClick={() => void requestContext("separated")}
            >
              Separate workspace and browser for this agent
            </button>
          </GroupedRow>
        ) : null}
      </GroupedList>
      <GroupedList
        title="Allowed capabilities"
        footer="Shell commands can read and write anything their OS user can access, even with file tools off. Saving changes cancels this agent’s current work on this machine."
      >
        <fieldset disabled={blocked || mutation.isPending} className="min-w-0">
          <legend className="sr-only">Allowed capabilities</legend>
          {(Object.keys(labels) as Array<keyof typeof labels>).map((key) => (
            <GroupedRow key={key} asChild interactive>
              <label>
                <span className="min-w-0 flex-1">
                  {labels[key]}
                  <span className="block text-11 font-normal text-text-tertiary">
                    {descriptions[key]}{!machine.ceiling[key] ? " · Off on node" : ""}
                  </span>
                </span>
                <Checkbox
                  aria-label={labels[key]}
                  checked={caps[key]}
                  disabled={
                    !machine.ceiling[key] ||
                    ((key === "computer" || key === "developer_browser") &&
                      !caps.browser)
                  }
                  onCheckedChange={(value) => {
                    form.setValue(`capabilities.${key}`, value === true);
                    if (key === "browser" && value !== true) { form.setValue("capabilities.computer", false); form.setValue("capabilities.developer_browser", false); }
                  }}
                />
              </label>
            </GroupedRow>
          ))}
        </fieldset>
      </GroupedList>
      <GroupedFormActions
          changes={() => [...recordChanges(form.formState.defaultValues?.capabilities, form.getValues("capabilities"), (key) => labels[key as keyof typeof labels] ?? key), ...describeChanges(form.formState.defaultValues, form.getValues(), [{ key: "saved_login_ids", label: "Saved logins", list: changeList }])]}
        label="machine access"
        dirty={form.formState.isDirty}
        pending={mutation.isPending}
        disabled={blocked}
        saveLabel="Save capabilities"
        save={save}
        discard={() => {
          form.reset();
          setError(undefined);
        }}
      />
    </form>
  );
}

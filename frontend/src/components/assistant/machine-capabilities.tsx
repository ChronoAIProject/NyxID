import { useId, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  machineSelectionSchema,
  type MachineAccess,
  type MachineSelection,
} from "@/schemas/machine-access";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import { MachineSeparationSupport } from "@/components/shared/machine-isolation";
import { useAppForm } from "@/components/ui/form";
import { Checkbox } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { hasPendingMachineApplication } from "@/lib/machine-access";
import {
  useMachineAccess,
  useSetMachineAccess,
} from "@/hooks/use-machine-access";
import { MachineAuthorizationStatus } from "./machine-authorization-status";
import { MachineContextReview } from "./machine-context-review";
import { MachineApplicationStatus } from "./machine-application-status";

const labels = {
  shell: "Shell commands",
  files: "File tools",
  browser: "Secure browser",
  computer: "Full computer control",
  developer_browser: "Developer browser",
} as const;
const capabilityKeys = Object.keys(labels) as Array<keyof typeof labels>;
type Membership = "owner" | "assigned" | "not_assigned";

export function MachineCapabilities({
  agentId,
  agent,
  disabled = false,
}: {
  readonly agentId: string;
  readonly agent?: Pick<AssistantAgent, "kind" | "machines">;
  readonly disabled?: boolean;
}) {
  const enabled = useFeature(FEATURE_FLAG.MACHINE_CAPABILITIES);
  const contextsEnabled = useFeature(FEATURE_FLAG.MACHINE_CONTEXTS);
  const query = useMachineAccess(agentId, !disabled);
  if (!enabled && !contextsEnabled && !query.data?.length) return null;
  return (
    <section aria-label="Machine capabilities" className="space-y-3">
      <p className="text-12 text-muted-foreground">
        Choose what this agent can do on each machine. Membership, saved tools
        and environment mode are separate settings. Existing restrictions still
        apply when editing is disabled. Saving changes requests cancellation of
        this agent’s current work on that machine.
      </p>
      {query.error ? (
        <p role="alert" className="text-12 text-destructive">
          Could not refresh machine capabilities. Displayed status may be out of
          date.
        </p>
      ) : null}
      {query.isPending ? (
        <p role="status" className="text-12 text-muted-foreground">
          Loading machine access…
        </p>
      ) : null}
      {query.data?.some(hasPendingMachineApplication) ? (
        <p className="text-12 text-muted-foreground">
          Status refreshes for up to one minute while this page is active. Use
          Refresh machine status for a later check.
        </p>
      ) : null}
      <Button
        type="button"
        size="sm"
        variant="secondary"
        disabled={query.isFetching || disabled}
        onClick={() => void query.refetch()}
      >
        Refresh machine status
      </Button>
      {query.data?.map((machine) => (
        <MachineCapabilityForm
          key={`${agentId}:${machine.node_id}`}
          machine={machine}
          agentId={agentId}
          membership={
            agent
              ? agent.kind === "nyxbot"
                ? "owner"
                : agent.machines?.includes(machine.node_id)
                  ? "assigned"
                  : "not_assigned"
              : undefined
          }
          disabled={disabled || Boolean(query.error)}
          capabilityEditing={enabled}
          contextEnabled={contextsEnabled}
        />
      ))}
    </section>
  );
}

export function MachineCapabilityForm({
  machine,
  agentId,
  membership,
  disabled = false,
  capabilityEditing = true,
  contextEnabled = false,
  onSaveError,
  onSaveSuccess,
}: {
  readonly machine: MachineAccess;
  readonly agentId: string;
  readonly membership?: Membership;
  readonly disabled?: boolean;
  readonly capabilityEditing?: boolean;
  readonly contextEnabled?: boolean;
  readonly onSaveError?: () => void;
  readonly onSaveSuccess?: () => void;
}) {
  const mutation = useSetMachineAccess(agentId);
  const descriptionId = useId();
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const needsSelection =
    machine.selection_required === true ||
    machine.mode === "selection_required";
  const separationAvailable =
    machine.separated?.available === true && !machine.separated.reason;
  const form = useAppForm<MachineSelection>({
    resolver: zodResolver(machineSelectionSchema),
    values: {
      expected_revision: machine.revision,
      capabilities: machine.capabilities,
      saved_login_ids: machine.saved_login_ids,
    },
  });
  const caps = form.watch("capabilities");
  const editReasons = [
    (!capabilityEditing || !machine.capability_edit_enabled) &&
      "Capability editing is not enabled for this account. Existing restrictions still apply.",
    needsSelection && "Choose an environment mode before editing capabilities.",
    !machine.can_edit &&
      "Only the machine owner or an organization admin can edit this access.",
    !machine.protocol_v2 &&
      "Update this machine before changing capabilities or environment mode. Its saved restrictions still apply.",
    disabled &&
      "This Agent is read-only or its current access could not be refreshed. Its saved permissions still apply.",
  ].filter((reason): reason is string => Boolean(reason));
  const busy = mutation.isPending;
  const capabilityDisabled = editReasons.length > 0 || busy;
  const savedTools = capabilityKeys
    .filter((key) => machine.capabilities[key])
    .map((key) => labels[key]);
  return (
    <form
      className="space-y-3 rounded-xl border border-border p-4"
      aria-label={`${machine.name} capabilities`}
      onSubmit={form.handleSubmit(async (selection) => {
        if (capabilityDisabled || !form.formState.isDirty) return;
        setError(undefined);
        setSaved(false);
        try {
          await mutation.mutateAsync({ node: machine.node_id, selection });
          // Reactive values may already contain a newer policy while the
          // mutation waits for other Agent queries. Keep those current defaults.
          form.reset();
          setSaved(true);
          onSaveSuccess?.();
        } catch {
          setError("Could not save capabilities. Reload if access changed.");
          onSaveError?.();
        }
      })}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-13 font-medium">{machine.name}</span>
        <Badge variant="secondary">
          {needsSelection
            ? "Owner selection required"
            : machine.mode === "separated"
              ? "Separate workspace and browser"
              : "Shared legacy"}
        </Badge>
      </div>
      {membership ? (
        <p className="text-12 text-muted-foreground">
          Membership:{" "}
          <span className="text-foreground">
            {membership === "owner"
              ? "Owner machine membership"
              : membership === "assigned"
                ? "Assigned"
                : "Not assigned"}
          </span>
          . Membership and tool grants are separate.
          {membership === "not_assigned"
            ? " Saving capabilities also assigns this machine to the agent."
            : ""}
        </p>
      ) : null}
      <p className="text-12">
        {savedTools.length
          ? `Saved tools: ${savedTools.join(", ")}`
          : "No tools granted. Membership alone does not authorize tools."}
      </p>
      <p className="text-12 text-muted-foreground">
        {needsSelection
          ? "Execution is blocked until the owner records a supported choice or explicitly approves legacy access."
          : machine.mode === "separated"
            ? "This agent is configured to use its own workspace and browser profiles. Use a separate machine container or VM per agent for stronger separation; containers still share their host kernel."
            : "Workspace and browser sessions are shared with other agents in shared legacy mode on this machine."}
      </p>
      <p className="text-12 text-muted-foreground">
        {machine.execution_note ??
          (needsSelection
            ? "Execution is blocked until the owner completes separation selection."
            : machine.mode === "separated"
              ? separationAvailable
                ? "Granted commands and file tools use this agent’s separated workspace. No shared fallback."
                : "This assignment requires separated mode, but separation is unavailable. Execution is refused. Restore support or ask the owner to change mode through the owner review below; no automatic shared fallback."
              : "Granted commands and file tools run in shared mode, as configured, even when separation is unavailable. Shell uses the command user’s full OS permissions.")}
      </p>
      <p className="text-12 text-muted-foreground">
        Saved permissions do not confirm runtime availability. Each execution
        checks current membership, grants, machine connection, runtime, funding
        and owner-turn requirements.
      </p>
      {needsSelection ? (
        <p role="status" className="text-12">
          {machine.selection_reason ??
            "Choose a supported environment mode. Missing capability evidence requires a machine update before separated access can be selected."}
        </p>
      ) : null}
      <MachineAuthorizationStatus machine={machine} />
      <MachineApplicationStatus authorizationApplied={machine.authorization_applied}
        environmentApplied={machine.environment_applied} configuration={machine.machine_configuration} />
      {machine.availability_reason ? <p role="status" className="text-12 text-warning">
        Agent tools unavailable: {({
          machine_selection_required: "owner environment selection required",
          no_tools_granted: "no tools granted; this can be an intentional all-off policy",
          machine_offline: "machine is offline",
          separated_context_unavailable: "separated support is unavailable; execution is refused without shared fallback",
          authority_application_pending: "Agent authority application is pending",
          machine_configuration_pending: "current hosted configuration evidence is pending",
        } as Record<string, string>)[machine.availability_reason] ?? machine.availability_reason.replaceAll("_", " ")}
      </p> : null}
      <MachineSeparationSupport
        support={machine.separated}
        setupNote={machine.separated_setup_note}
      />
      <div
        id={`${descriptionId}-editing`}
        className="space-y-1 text-12 text-muted-foreground"
      >
        {editReasons.map((reason) => (
          <p key={reason} role="status">
            {reason}
          </p>
        ))}
      </div>
      <MachineContextReview
        machine={machine}
        agentId={agentId}
        contextEnabled={contextEnabled}
        disabled={disabled || mutation.isPending}
        unsavedCapabilities={form.formState.isDirty}
      />
      <fieldset
        disabled={capabilityDisabled}
        className="grid gap-2 sm:grid-cols-2"
      >
        <legend className="sr-only">Allowed capabilities</legend>
        {capabilityKeys.map((key) => {
          const reason = !machine.ceiling[key]
            ? "Off on this machine. The machine capability ceiling does not allow this tool."
            : (key === "computer" || key === "developer_browser") &&
                !caps.browser
              ? "Enable Secure browser for this agent first."
              : undefined;
          return (
            <div key={key} className="space-y-1 text-12">
              <label className="flex items-center gap-2">
                <Checkbox
                  aria-label={labels[key]}
                  aria-describedby={`${descriptionId}-editing${reason ? ` ${descriptionId}-${key}` : ""}`}
                  checked={caps[key]}
                  disabled={capabilityDisabled || Boolean(reason)}
                  onCheckedChange={(value) => {
                    setSaved(false);
                    form.setValue(`capabilities.${key}`, value === true);
                    if (key === "browser" && value !== true) {
                      form.setValue("capabilities.computer", false);
                      form.setValue("capabilities.developer_browser", false);
                    }
                  }}
                />
                {labels[key]}
              </label>
              {reason ? (
                <p
                  id={`${descriptionId}-${key}`}
                  className="text-muted-foreground"
                >
                  {reason}
                </p>
              ) : null}
            </div>
          );
        })}
      </fieldset>
      <details className="text-12 text-muted-foreground">
        <summary className="cursor-pointer">
          What these permissions mean
        </summary>
        <p className="mt-2">
          Shell commands can read and write anything their OS user can access,
          even with file tools off. Full computer control can operate
          applications through their UI. Browser-only access avoids desktop
          authority. Developer browser permits JavaScript evaluation in its
          separate browser; saved logins stay in the secure browser.
        </p>
      </details>
      {saved ? (
        <p role="status" className="text-12 text-muted-foreground">
          Capabilities saved. Saving does not confirm machine acknowledgement,
          browser cleanup or hosted environment application. Refresh machine
          status to check the Agent authority chain.
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="text-12 text-destructive">
          {error}
        </p>
      ) : null}
      <div className="flex justify-end">
        <Button
          type="submit"
          size="sm"
          variant="primary"
          disabled={capabilityDisabled || !form.formState.isDirty}
          isLoading={mutation.isPending}
        >
          Save capabilities
        </Button>
      </div>
    </form>
  );
}

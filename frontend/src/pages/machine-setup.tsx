import { useRemoteMachineAdmission } from "@/hooks/use-remote-machine-admission";
import { RemoteMachineCreate } from "@/components/machines/remote-machine-create";
import { MachineSeparationSupport } from "@/components/shared/machine-isolation";
import { useVerifiedUpdaterImage } from "@/hooks/use-machines";
import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
import { WorkspacePageHeader as PageHeader } from "@/components/assistant/workspace-page-header";
import { CopyableField } from "@/components/shared/copyable-field";
import { Button } from "@/components/ui/button";
import {
  GroupedInput as Input,
  GroupedList,
  GroupedRow,
  GroupedFooter,
  GroupedSaveBar,
  GroupedSelectTrigger as SelectTrigger,
} from "@/components/ui/grouped-list";
import {
  GroupedField as FormItem,
  GroupedLabel as FormLabel,
  GroupedMessage as FormMessage,
} from "@/components/ui/grouped-fields";
import { GroupedDialogActions } from "@/components/ui/grouped-form";
import { GroupedPage, GroupedDecisionDraft } from "@/components/ui/grouped-dialog";
import { useGroupedDraftGuard } from "@/lib/grouped-drafts";
import { Checkbox } from "@/components/ui/checkbox";
import { Switch } from "@/components/ui/switch";
import {
  SegmentedControl as Tabs,
  SegmentedControlList as TabsList,
  SegmentedControlTrigger as TabsTrigger,
} from "@/components/ui/segmented-control";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import { Form, FormField, FormControl, useAppForm } from "@/components/ui/form";
import { usePublicConfig } from "@/hooks/use-public-config";
import { useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import {
  useMachineSetup,
  useCreateMachineSetup,
  issueMachineSetup,
  useMachinePairPreview,
  useMachinePairDecision,
} from "@/hooks/use-machines";
import {
  MACHINE_SAFETY,
  SHARED_LEGACY_WARNING,
  SINGLE_USER_SHELL_WARNING,
  SINGLE_USER_WARNING,
  machineChoicesSchema,
  machineSetupCommand,
  type MachineChoices,
  type MachineSetup,
} from "@/schemas/machines";

const labels = {
  shell: "Commands — run code and use connected services",
  files: "Files — read, edit and move files in workspace roots",
  browser: "Browser — use the managed browser",
  computer: "Computer — operate the desktop and browser",
};
const states: Record<string, string> = {
  review: "Review your choices",
  selection_required: "Connected, but machine access requires an owner separation choice. Review Assistant → Machines, or rerun setup with an explicit choice.",
  waiting: "Waiting for the machine to connect…",
  approved: "Approved. Waiting for the machine…",
  connected: "Connected. NyxBot can verify the machine and continue.",
  expired: "Setup expired. Start a new setup to get a fresh command.",
  declined: "Pairing declined.",
  failed:
    "Setup failed. Start a new setup and check the machine's local status.",
  permissions_missing:
    "Connected, but computer permissions are missing. Run nyxid node machine status on the machine.",
  offline: "The machine is offline. Start its node daemon to reconnect.",
};

export function MachineSetupPage() {
  return (
    <GroupedPage>
      <MachineSetupContent />
    </GroupedPage>
  );
}

function MachineSetupContent() {
  const guard = useGroupedDraftGuard();
  const search = useSearch({ strict: false }) as {
    setup?: string;
    where?: "hosted";
  };
  const navigate = useNavigate();
  const remoteAdmission = useRemoteMachineAdmission();
  const [hostedSelected, setHosted] = useState(
    search.where === "hosted" && !search.setup,
  );
  const hosted = hostedSelected && remoteAdmission;
  const [id, setId] = useState(search.setup);
  const setup = useMachineSetup(id);
  const create = useCreateMachineSetup();
  const config = usePublicConfig();
  const agents = useNyxBotAgents();
  const [command, setCommand] = useState<string>();
  const [issuing, setIssuing] = useState(false);
  const [error, setError] = useState<string>();
  const form = useAppForm<MachineChoices>({
    resolver: zodResolver(machineChoicesSchema),
    defaultValues: {
      separation: "automatic",
      legacy_warning_acknowledged: false,
      name: "my-machine",
      where: "vm",
      capabilities: ["shell", "files"],
      grant_to: null,
      automatic_updates: true,
    },
  });
  const prefilled = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (
      setup.data?.status === "review" &&
      prefilled.current !== setup.data.id
    ) {
      prefilled.current = setup.data.id;
      form.reset({
        ...setup.data.choices,
        separation: setup.data.choices.separation ?? "automatic",
        legacy_warning_acknowledged: false,
        automatic_updates: setup.data.choices.automatic_updates ?? true,
      });
    }
  }, [setup.data, form]);
  useEffect(() => {
    if (
      setup.data &&
      ["connected", "expired", "failed"].includes(setup.data.status)
    )
      setCommand(undefined);
  }, [setup.data]);
  const updater = useVerifiedUpdaterImage();
  const where = form.watch("where");
  const capabilities = form.watch("capabilities");
  const locked =
    issuing ||
    Boolean(command) ||
    Boolean(setup.data && setup.data.status !== "review");
  async function submit(choices: MachineChoices) {
    setError(undefined);
    setIssuing(true);
    try {
      if (choices.where === "docker" && !config.data?.version)
        throw new Error(
          "Server release version is unavailable. Reload before creating the Docker command.",
        );
      if (
        choices.where === "docker" &&
        (!updater.data?.image || updater.data.version !== config.data?.version)
      )
        throw new Error("Verifying the updater image, try again shortly.");
      const row = id ? { id } : await create.mutateAsync(choices);
      setId(row.id);
      const issued = await issueMachineSetup(row.id, choices);
      const wsUrl =
        config.data?.node_ws_url ??
        `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}/api/v1/nodes/ws`;
      setCommand(
        machineSetupCommand(
          choices,
          issued.token,
          wsUrl,
          config.data?.version,
          updater.data?.image ?? undefined,
        ),
      );
      issued.token = "";
      form.reset(choices);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not create setup command",
      );
    } finally {
      setIssuing(false);
    }
  }
  return (
    <div
      className="ph-no-capture max-w-2xl space-y-6"
      data-private="true"
      data-ph-no-capture
    >
      <PageHeader
        title="Add a machine"
        description={
          hosted
            ? "Configure a personal NyxID hosted machine and review its billing and data policies."
            : "Review the choices, then run one command on the machine."
        }
      />
      {!hosted && <SetupSafety />}
      <Tabs
        value={hosted ? "hosted" : where}
        onValueChange={async (value) => {
          if (
            (value === "hosted") !== Boolean(hosted) &&
            guard &&
            !(await guard.confirmDiscard())
          )
            return;
          setHosted(value === "hosted");
          if (value !== "hosted")
            form.setValue("where", value as MachineChoices["where"]);
        }}
      >
        <TabsList>
          <TabsTrigger
            value="this_computer"
            disabled={locked}
            className="px-2 sm:px-3"
          >
            This computer
          </TabsTrigger>
          <TabsTrigger value="vm" disabled={locked} className="px-2 sm:px-3">
            Remote VM
          </TabsTrigger>
          <TabsTrigger
            value="docker"
            disabled={locked}
            className="px-2 sm:px-3"
          >
            Docker
          </TabsTrigger>
          {remoteAdmission && (
            <TabsTrigger
              value="hosted"
              disabled={locked}
              className="px-2 sm:px-3"
            >
              NyxID hosted
            </TabsTrigger>
          )}
        </TabsList>
      </Tabs>
      {hosted ? (
        <RemoteMachineCreate
          onCancel={() => void navigate({ to: "/assistant/machines" })}
          onCreated={(machineId, operationId) =>
            void navigate({
              to: "/assistant/machines",
              search: { machine: machineId, operation: operationId },
            })
          }
        />
      ) : (
        <Form {...form}>
          <form onSubmit={form.handleSubmit(submit)} className="space-y-6">
            <GroupedList title="Machine">
              <FormField
                control={form.control}
                name="owner_id"
                render={({ field }) => (
                  <FormItem inline>
                    <FormLabel>Machine owner</FormLabel>
                    <FormControl>
                      <OrgScopeSelect
                        grouped
                        value={field.value ?? null}
                        onChange={field.onChange}
                        disabled={locked}
                        label="Machine owner"
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Machine name</FormLabel>
                    <FormControl>
                      <Input {...field} disabled={locked} autoComplete="off" />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </GroupedList>
            <GroupedList
              title="Agent separation"
              footer={
                <>
                  <p>
                    Automatic and required refuse when the real host probes
                    cannot confirm support. macOS and browserless nodes need an
                    informed legacy choice or a supported Linux host.
                  </p>
                  {where !== "docker" && (
                    <p>
                      On Linux, provision distinct OS users by running the
                      generated setup command with sudo and --separate-users.
                      Separation also needs a managed browser; enable Browser
                      only if you want to grant that capability. The command
                      preserves your selected capabilities.
                    </p>
                  )}
                  {form.watch("separation") === "shared_legacy" && (
                    <p className="text-warning">{SHARED_LEGACY_WARNING}</p>
                  )}
                </>
              }
            >
              <FormField
                control={form.control}
                name="separation"
                render={({ field }) => (
                  <FormItem inline>
                    <FormLabel className="flex-1 text-12 font-medium text-foreground">
                      Agent separation
                    </FormLabel>
                    <Select value={field.value ?? "automatic"} disabled={locked} onValueChange={value => {
                      // The hidden select can emit an empty value during reset.
                      // Preserve the reviewed choice until a valid option is selected.
                      if (!["automatic", "require_separated", "shared_legacy"].includes(value)) return;
                      field.onChange(value);
                      form.setValue("legacy_warning_acknowledged", false);
                    }}>
                      <FormControl><SelectTrigger><SelectValue /></SelectTrigger></FormControl>
                      <SelectContent>
                        <SelectItem value="automatic">Automatic — require confirmed supported separation</SelectItem>
                        <SelectItem value="require_separated">Require separated workspaces and browsers</SelectItem>
                        <SelectItem value="shared_legacy">Explicit shared legacy access</SelectItem>
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
              {form.watch("separation") === "shared_legacy" && (
                <GroupedRow asChild>
                  <label>
                    <span className="flex-1">
                      I explicitly accept shared legacy access and this warning.
                    </span>
                    <Switch disabled={locked} checked={form.watch("legacy_warning_acknowledged") === true} onCheckedChange={value => form.setValue("legacy_warning_acknowledged", value === true)} />
                    <FormField control={form.control} name="legacy_warning_acknowledged" render={() => <FormMessage />} />
                  </label>
                </GroupedRow>
              )}
            </GroupedList>
            <GroupedList title="Enable on this machine">
              {(["shell", "files", "browser", "computer"] as const).map(
                (capability) => (
                  <GroupedRow key={capability} asChild>
                    <label>
                      <span className="flex-1">{labels[capability]}</span>
                      <Checkbox
                        disabled={locked}
                        checked={capabilities.includes(capability)}
                        onCheckedChange={(checked) =>
                          form.setValue(
                            "capabilities",
                            checked
                              ? [...capabilities, capability]
                              : capabilities.filter((c) => c !== capability),
                          )
                        }
                      />
                    </label>
                  </GroupedRow>
                ),
              )}
            </GroupedList>
            <GroupedList
              title="Specialist access"
              footer="Agent permissions are configured separately in agent settings. New assignments start with no allowed capabilities."
            >
              <FormField
                control={form.control}
                name="grant_to"
                render={({ field }) => (
                  <FormItem inline>
                    <FormLabel>Select a specialist for this machine</FormLabel>
                    <Select
                      value={field.value ?? "none"}
                      disabled={locked}
                      onValueChange={(value) =>
                        field.onChange(value === "none" ? null : value)
                      }
                    >
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="none">
                          No specialist assignment
                        </SelectItem>
                        {agents.data?.agents
                          .filter(
                            (agent) =>
                              agent.kind !== "nyxbot" &&
                              agent.status !== "destroyed",
                          )
                          .map((agent) => (
                            <SelectItem key={agent.id} value={agent.id}>
                              {agent.name}
                            </SelectItem>
                          ))}
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </GroupedList>
            <GroupedList
              title="Updates"
              footer={
                <>
                  <p>
                    Update automatically when idle (recommended). Updates wait
                    for work and desktop sessions to end, and notify you.
                  </p>
                  {where === "docker" && (
                    <p>
                      The command also installs a small updater companion. Its
                      Docker socket grants host-root access; only the node
                      supervisor can request an update, and the helper accepts
                      only a release version, verifies the official image
                      attestation and restores the previous container if
                      reconnection fails. Your identity and workspace volumes
                      are preserved.
                    </p>
                  )}
                  {where !== "docker" &&
                    (capabilities.includes("computer") ||
                      capabilities.includes("browser")) && (
                      <p>
                        {SINGLE_USER_WARNING} Saved-login typing starts off;
                        allow it later in Nodes settings. Setup asks once for
                        administrator access to install browser policies. On
                        macOS, also allow Screen Recording and Accessibility.
                      </p>
                    )}
                </>
              }
            >
              <GroupedRow asChild>
                <label>
                  <Switch
                    className="order-last shrink-0"
                    checked={form.watch("automatic_updates") ?? true}
                    disabled={locked}
                    onCheckedChange={(value) =>
                      form.setValue("automatic_updates", value === true)
                    }
                  />
                  <span className="flex-1">Update automatically when idle</span>
                </label>
              </GroupedRow>
            </GroupedList>
            {!locked && (
              <GroupedDialogActions
                intent="create"
                cancel={() => void navigate({ to: "/assistant/machines" })}
                label="machine setup"
                dirty={form.formState.isDirty}
                pending={issuing}
                disabled={!capabilities.length || !form.watch("name")}
                save={form.handleSubmit(submit)}
                discard={() => {
                  form.reset();
                  setError(undefined);
                }}
                saveLabel="Create my setup command"
              />
            )}
          </form>
        </Form>
      )}
      {!hosted && command ? (
        <GroupedList
          title="Run on your machine"
          footer="Single-use command, shown only here. Do not paste it into chat. It expires in 15 minutes."
        >
          <GroupedRow>
            <CopyableField
              grouped
              label="Run on your machine"
              value={command}
            />
          </GroupedRow>
        </GroupedList>
      ) : null}
      {setup.data ? <SetupProgress setup={setup.data} /> : null}
      {error || setup.error ? (
        <GroupedList
          footer={
            <p role="alert" className="text-destructive">
              {error ?? setup.error?.message}
            </p>
          }
        />
      ) : null}
    </div>
  );
}

export function MachinePairPage() {
  return <GroupedPage><MachinePairContent /></GroupedPage>;
}

function MachinePairContent() {
  const navigate = useNavigate();
  const search = useSearch({ strict: false }) as { code?: string };
  const [code, setCode] = useState(search.code ?? "");
  const [details, setDetails] = useState<MachineSetup>();
  const [confirmed, setConfirmed] = useState(false);
  const [automatic, setAutomatic] = useState(true);
  const preview = useMachinePairPreview();
  const decision = useMachinePairDecision();
  const status = useMachineSetup(decision.data?.id);
  async function review(event: React.FormEvent) {
    event.preventDefault();
    setDetails(await preview.mutateAsync(code));
    setConfirmed(false);
  }
  async function decide(approve: boolean) {
    await decision.mutateAsync({ code, approve, automatic_updates: automatic });
  }
  return (
    <div className="max-w-xl space-y-6">
      <GroupedDecisionDraft
        dirty={confirmed && !decision.data}
        pending={decision.isPending}
        discard={() => setConfirmed(false)}
      />
      <PageHeader
        title="Pair this machine"
        description="Approve only a setup you started."
      />
      <form
        onSubmit={(event) => {
          void review(event).catch(() => undefined);
        }}
        className="space-y-6"
      >
        <GroupedList title="Pairing code">
          <GroupedRow>
            <Input
              aria-label="Pairing code"
              value={code}
              maxLength={16}
              disabled={Boolean(decision.data) || preview.isPending || decision.isPending}
              onChange={(event) => {
                setCode(event.target.value);
                setDetails(undefined);
              }}
            />
            <Button
              variant="ghost"
              type="submit"
              isLoading={preview.isPending}
              disabled={!code || Boolean(decision.data) || preview.isPending || decision.isPending}
            >
              Review
            </Button>
          </GroupedRow>
        </GroupedList>
      </form>
      {details ? (
        <div className="space-y-6">
          <GroupedList title="Machine" footer={MACHINE_SAFETY}>
            <GroupedRow title="Hostname" accessory={details.hostname} />
            <GroupedRow title="Operating system" accessory={details.os} />
            <GroupedRow title="IP address" accessory={details.ip} />
            <GroupedRow
              title="Requested access"
              accessory={details.choices.capabilities.join(", ")}
            />
          </GroupedList>
          <GroupedList
            title="Pairing"
            footer="Automatic updates wait for jobs, turns and live desktop sessions to end, and notify you."
          >
            <GroupedRow asChild>
              <label>
                <Switch
                  className="order-last shrink-0"
                  checked={automatic}
                  disabled={Boolean(decision.data) || decision.isPending}
                  onCheckedChange={(value) => setAutomatic(value === true)}
                />
                <span className="flex-1">
                  Update automatically when idle (recommended)
                </span>
              </label>
            </GroupedRow>
            {!decision.data ? (
              <>
                <GroupedRow asChild>
                  <label>
                    <Switch
                      className="order-last shrink-0"
                      checked={confirmed}
                      disabled={decision.isPending}
                      onCheckedChange={(value) => setConfirmed(value === true)}
                    />
                    <span className="flex-1">
                      I started this setup and recognize this machine.
                    </span>
                  </label>
                </GroupedRow>
              </>
            ) : (
              <GroupedRow>
                <p role="status">
                  {states[status.data?.status ?? decision.data.status] ??
                    "Waiting for the machine…"}
                </p>
              </GroupedRow>
            )}
          </GroupedList>
        </div>
      ) : null}
      {details && !decision.data ? (
        <GroupedSaveBar aria-label="Pairing actions">
          <Button variant="ghost" disabled={decision.isPending} onClick={() => void navigate({ to: "/assistant/machines" })}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            disabled={decision.isPending}
            onClick={() => void decide(false).catch(() => undefined)}
          >
            Decline
          </Button>
          <Button
            variant="primary"
            disabled={!confirmed || decision.isPending}
            isLoading={decision.isPending}
            onClick={() => void decide(true).catch(() => undefined)}
          >
            Approve pairing
          </Button>
        </GroupedSaveBar>
      ) : null}
      {preview.error || decision.error ? (
        <GroupedList
          footer={
            <p role="alert" className="text-destructive">
              {(preview.error ?? decision.error)?.message}
            </p>
          }
        />
      ) : null}
    </div>
  );
}

function SetupSafety() {
  return (
    <GroupedList
      title="Before you start"
      footer={
        <>
          <p>{SINGLE_USER_SHELL_WARNING}</p>
          <p>
            The Docker command downloads NyxID’s seccomp profile and passes
            --security-opt seccomp to enable Chromium’s user-namespace sandbox.
            It grants no additional container capabilities.
          </p>
          <p>
            The image is pinned to this server’s release. After a server
            upgrade, recreate the container with the matching image and the same
            identity and workspace volumes to keep this machine connected.
          </p>
        </>
      }
    >
      <GroupedRow title="Review machine access" />
      <GroupedFooter>
        <p className="text-warning">{MACHINE_SAFETY}</p>
      </GroupedFooter>
    </GroupedList>
  );
}

function SetupProgress({ setup }: { readonly setup: MachineSetup }) {
  return (
    <GroupedList
      title="Setup progress"
      role="status"
      footer={
        <>
          {setup.selection_reason && !/^[a-z][a-z_]+$/.test(setup.selection_reason) && (
            <p>{setup.selection_reason.replace(/^[a-z_]+:\s*/, "")}</p>
          )}
          {setup.machine && (
            <MachineSeparationSupport humanOnly machine={setup.machine} />
          )}
        </>
      }
    >
      <GroupedRow title={states[setup.status] ?? "Waiting for setup…"} />
      {setup.status === "selection_required" && (
        <GroupedRow asChild interactive>
          <Link to="/assistant/machines">Review machine separation</Link>
        </GroupedRow>
      )}
      {setup.machine && (
        <GroupedRow
          title="Enabled"
          accessory={(["shell", "files", "browser", "computer"] as const)
            .filter((c) => setup.machine?.[c])
            .join(", ")}
        />
      )}
      <GroupedRow asChild interactive>
        {setup.conversation_id ? (
          <Link to="/assistant" search={{ c: setup.conversation_id }}>
            Back to NyxBot
          </Link>
        ) : (
          <Link to="/nodes">View nodes</Link>
        )}
      </GroupedRow>
    </GroupedList>
  );
}

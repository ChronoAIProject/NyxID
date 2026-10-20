import { useRemoteMachineAdmission } from "@/hooks/use-remote-machine-admission";
import { RemoteMachineCreate } from "@/components/machines/remote-machine-create";
import { MachineSeparationSupport } from "@/components/shared/machine-isolation";
import { useVerifiedUpdaterImage } from "@/hooks/use-machines";
import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
import { PageHeader } from "@/components/shared/page-header";
import { CopyableField } from "@/components/shared/copyable-field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Form,
  FormField,
  FormItem,
  FormLabel,
  FormControl,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
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
      {hosted ? (
        <p className="rounded-lg border border-warning/30 bg-warning/5 p-4 text-12 text-muted-foreground">
          Agents can run code and access this Machine’s workspace. Prompt
          injection is possible. All authorized clients share the OS and
          workspace; file roots do not isolate shell commands.
        </p>
      ) : (
        <SetupSafety />
      )}
      <Tabs
        value={hosted ? "hosted" : where}
        onValueChange={(value) => {
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
          onCreated={(machineId, operationId) =>
            void navigate({
              to: "/assistant/machines",
              search: { machine: machineId, operation: operationId },
            })
          }
        />
      ) : (
        <Form {...form}>
          <form onSubmit={form.handleSubmit(submit)} className="space-y-4">
            <FormField
              control={form.control}
              name="owner_id"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Machine owner</FormLabel>
                  <FormControl>
                    <OrgScopeSelect
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
            <FormField control={form.control} name="separation" render={({ field }) => <FormItem>
              <FormLabel>Agent separation</FormLabel>
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
              <p className="text-12 text-muted-foreground">Automatic and required refuse when the real host probes cannot confirm support. macOS and browserless nodes need an informed legacy choice or a supported Linux host.</p>
              {where !== "docker" ? <p className="text-12 text-muted-foreground">On Linux, provision distinct OS users by running the generated setup command with sudo and --separate-users. Separation also needs a managed browser; enable Browser only if you want to grant that capability. The command preserves your selected capabilities.</p> : null}
              <FormMessage />
            </FormItem>} />
            {form.watch("separation") === "shared_legacy" && <div className="space-y-2 rounded-lg border p-3 text-12">
              <p>{SHARED_LEGACY_WARNING}</p>
              <label className="flex items-start gap-2"><Checkbox disabled={locked} checked={form.watch("legacy_warning_acknowledged") === true} onCheckedChange={value => form.setValue("legacy_warning_acknowledged", value === true)} />I explicitly accept shared legacy access and this warning.</label>
              <FormField control={form.control} name="legacy_warning_acknowledged" render={() => <FormMessage />} />
            </div>}
            <fieldset disabled={locked} className="space-y-3">
              <legend className="mb-2 text-12 font-medium">
                Enable on this machine
              </legend>
              {(["shell", "files", "browser", "computer"] as const).map(
                (capability) => (
                  <label
                    key={capability}
                    className="flex items-center gap-2 text-12"
                  >
                    <Checkbox
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
                    {labels[capability]}
                  </label>
                ),
              )}
            </fieldset>
            <FormField
              control={form.control}
              name="grant_to"
              render={({ field }) => (
                <FormItem>
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
            <p className="text-12 text-muted-foreground">
              Agent permissions are configured separately in agent settings. New
              assignments start with no allowed capabilities.
            </p>
            <label className="flex items-start gap-2 text-12">
              <Checkbox
                checked={form.watch("automatic_updates") ?? true}
                disabled={locked}
                onCheckedChange={(value) =>
                  form.setValue("automatic_updates", value === true)
                }
              />
              <span>
                Update automatically when idle (recommended). Updates wait for
                work and desktop sessions to end, and notify you.
              </span>
            </label>
            {where === "docker" ? (
              <p className="text-12 text-muted-foreground">
                The command also installs a small updater companion. Its Docker
                socket grants host-root access; only the node supervisor can
                request an update, and the helper accepts only a release
                version, verifies the official image attestation and restores
                the previous container if reconnection fails. Your identity and
                workspace volumes are preserved.
              </p>
            ) : null}
            {where !== "docker" &&
            (capabilities.includes("computer") ||
              capabilities.includes("browser")) ? (
              <p className="text-12 text-muted-foreground">
                {SINGLE_USER_WARNING} Saved-login typing starts off; allow it
                later in Nodes settings. Setup asks once for administrator
                access to install browser policies. On macOS, also allow Screen
                Recording and Accessibility.
              </p>
            ) : null}
            {!locked ? (
              <Button
                type="submit"
                variant="primary"
                isLoading={issuing}
                disabled={!capabilities.length || !form.watch("name")}
              >
                Create my setup command
              </Button>
            ) : null}
          </form>
        </Form>
      )}
      {!hosted && command ? (
        <div className="space-y-2">
          <CopyableField label="Run on your machine" value={command} />
          <p className="text-11 text-text-tertiary">
            Single-use command, shown only here. Do not paste it into chat. It
            expires in 15 minutes.
          </p>
        </div>
      ) : null}
      {setup.data ? <SetupProgress setup={setup.data} /> : null}
      {error || setup.error ? (
        <p role="alert" className="text-12 text-destructive">
          {error ?? setup.error?.message}
        </p>
      ) : null}
    </div>
  );
}

export function MachinePairPage() {
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
      <PageHeader
        title="Pair this machine"
        description="Approve only a setup you started."
      />
      <form
        onSubmit={(event) => {
          void review(event).catch(() => undefined);
        }}
        className="flex gap-2"
      >
        <Input
          aria-label="Pairing code"
          value={code}
          maxLength={16}
          disabled={Boolean(decision.data) || preview.isPending}
          onChange={(event) => {
            setCode(event.target.value);
            setDetails(undefined);
          }}
        />
        <Button
          type="submit"
          isLoading={preview.isPending}
          disabled={!code || Boolean(decision.data)}
        >
          Review
        </Button>
      </form>
      {details ? (
        <div className="space-y-4 rounded-xl border border-border bg-card p-4 text-12">
          <dl className="grid grid-cols-2 gap-2">
            <dt>Hostname</dt>
            <dd>{details.hostname}</dd>
            <dt>Operating system</dt>
            <dd>{details.os}</dd>
            <dt>IP address</dt>
            <dd>{details.ip}</dd>
            <dt>Requested access</dt>
            <dd>{details.choices.capabilities.join(", ")}</dd>
          </dl>
          <p className="text-muted-foreground">{MACHINE_SAFETY}</p>
          <label className="flex items-start gap-2">
            <Checkbox
              checked={automatic}
              disabled={Boolean(decision.data)}
              onCheckedChange={(value) => setAutomatic(value === true)}
            />
            <span>
              Update automatically when idle (recommended). Waits for jobs,
              turns and live desktop sessions to end, and notifies you.
            </span>
          </label>
          {!decision.data ? (
            <>
              <label className="flex items-center gap-2">
                <Checkbox
                  checked={confirmed}
                  onCheckedChange={(value) => setConfirmed(value === true)}
                />
                I started this setup and recognize this machine.
              </label>
              <div className="flex gap-2">
                <Button
                  variant="primary"
                  disabled={!confirmed}
                  isLoading={decision.isPending}
                  onClick={() => void decide(true).catch(() => undefined)}
                >
                  Approve pairing
                </Button>
                <Button
                  variant="destructive"
                  disabled={decision.isPending}
                  onClick={() => void decide(false).catch(() => undefined)}
                >
                  Decline
                </Button>
              </div>
            </>
          ) : (
            <p role="status">
              {states[status.data?.status ?? decision.data.status] ??
                "Waiting for the machine…"}
            </p>
          )}
        </div>
      ) : null}
      {preview.error || decision.error ? (
        <p role="alert" className="text-12 text-destructive">
          {(preview.error ?? decision.error)?.message}
        </p>
      ) : null}
    </div>
  );
}

function SetupSafety() {
  return (
    <div className="space-y-3">
      <p className="rounded-lg border border-warning/30 bg-warning/5 p-4 text-12 text-muted-foreground">
        {MACHINE_SAFETY}
      </p>
      <p className="text-12 text-muted-foreground">
        {SINGLE_USER_SHELL_WARNING}
      </p>
      <p className="text-12 text-muted-foreground">
        The Docker command downloads NyxID’s seccomp profile and passes
        --security-opt seccomp to enable Chromium’s user-namespace sandbox. It
        grants no additional container capabilities.
      </p>
      <p className="text-12 text-muted-foreground">
        The image is pinned to this server’s release. After a server upgrade,
        recreate the container with the matching image and the same identity and
        workspace volumes to keep this machine connected.
      </p>
    </div>
  );
}

function SetupProgress({ setup }: { readonly setup: MachineSetup }) {
  return (
    <div
      role="status"
      className="space-y-2 rounded-xl border border-border p-4 text-12"
    >
      <p>{states[setup.status] ?? "Waiting for setup…"}</p>
      {setup.status === "selection_required" && <><p>{setup.selection_reason}</p><Link to="/assistant/machines">Review machine separation</Link></>}
      {setup.machine ? (
        <><p>
          Enabled:{" "}
          {(["shell", "files", "browser", "computer"] as const)
            .filter((c) => setup.machine?.[c])
            .join(", ")}
        </p><MachineSeparationSupport machine={setup.machine} /></>
      ) : null}
      {setup.conversation_id ? (
        <Link to="/assistant" search={{ c: setup.conversation_id }}>
          Back to NyxBot
        </Link>
      ) : (
        <Link to="/nodes">View nodes</Link>
      )}
    </div>
  );
}

import { useVerifiedUpdaterImage } from "@/hooks/use-machines";
import { useEffect, useRef, useState } from "react";
import { Link, useSearch } from "@tanstack/react-router";
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
  computer: "Computer — operate the desktop and browser",
};
const states: Record<string, string> = {
  review: "Review your choices",
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
  const search = useSearch({ strict: false }) as { setup?: string };
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
      if (choices.where === "docker" && (!updater.data?.image || updater.data.version !== config.data?.version))
        throw new Error("Verifying the updater image, try again shortly.");
      const row = id ? { id } : await create.mutateAsync(choices);
      setId(row.id);
      const issued = await issueMachineSetup(row.id, choices);
      const wsUrl =
        config.data?.node_ws_url ??
        `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}/api/v1/nodes/ws`;
      setCommand(
        machineSetupCommand(choices, issued.token, wsUrl, config.data?.version, updater.data?.image ?? undefined),
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
        description="Review the choices, then run one command on the machine."
      />
      <SetupSafety />
      <Form {...form}>
        <form onSubmit={form.handleSubmit(submit)} className="space-y-4">
          <Tabs
            value={where}
            onValueChange={(value) =>
              form.setValue("where", value as MachineChoices["where"])
            }
          >
            <TabsList>
              <TabsTrigger value="this_computer" disabled={locked}>
                This computer
              </TabsTrigger>
              <TabsTrigger value="vm" disabled={locked}>
                Remote VM
              </TabsTrigger>
              <TabsTrigger value="docker" disabled={locked}>
                Docker
              </TabsTrigger>
            </TabsList>
          </Tabs>
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
          <fieldset disabled={locked} className="space-y-3">
            <legend className="mb-2 text-[12px] font-medium">
              Let agents use
            </legend>
            {(["shell", "files", "computer"] as const).map((capability) => (
              <label
                key={capability}
                className="flex items-center gap-2 text-[12px]"
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
            ))}
          </fieldset>
          <FormField
            control={form.control}
            name="grant_to"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Also let a specialist use this machine</FormLabel>
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
                    <SelectItem value="none">NyxBot only</SelectItem>
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
          <label className="flex items-start gap-2 text-[12px]">
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
            <p className="text-[12px] text-muted-foreground">
              The command also installs a small updater companion. Its Docker
              socket grants host-root access; only the node supervisor can
              request an update, and the helper accepts only a release version,
              verifies the official image attestation and restores the previous
              container if reconnection fails. Your identity and workspace
              volumes are preserved.
            </p>
          ) : null}
          {where !== "docker" && capabilities.includes("computer") ? (
            <p className="text-[12px] text-muted-foreground">
              {SINGLE_USER_WARNING} Saved-login typing starts off; allow it
              later in Nodes settings. Setup asks once for administrator access
              to install browser policies. On macOS, also allow Screen Recording
              and Accessibility.
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
      {command ? (
        <div className="space-y-2">
          <CopyableField label="Run on your machine" value={command} />
          <p className="text-[11px] text-text-tertiary">
            Single-use command, shown only here. Do not paste it into chat. It
            expires in 15 minutes.
          </p>
        </div>
      ) : null}
      {setup.data ? <SetupProgress setup={setup.data} /> : null}
      {error || setup.error ? (
        <p role="alert" className="text-[12px] text-destructive">
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
        <div className="space-y-4 rounded-xl border border-border bg-card p-4 text-[12px]">
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
        <p role="alert" className="text-[12px] text-destructive">
          {(preview.error ?? decision.error)?.message}
        </p>
      ) : null}
    </div>
  );
}

function SetupSafety() {
  return (
    <div className="space-y-3">
      <p className="rounded-lg border border-warning/30 bg-warning/5 p-4 text-[12px] text-muted-foreground">
        {MACHINE_SAFETY}
      </p>
      <p className="text-[12px] text-muted-foreground">
        {SINGLE_USER_SHELL_WARNING}
      </p>
      <p className="text-[12px] text-muted-foreground">
        The Docker command downloads NyxID’s seccomp profile and passes
        --security-opt seccomp to enable Chromium’s user-namespace sandbox. It
        grants no additional container capabilities.
      </p>
      <p className="text-[12px] text-muted-foreground">
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
      className="space-y-2 rounded-xl border border-border p-4 text-[12px]"
    >
      <p>{states[setup.status] ?? "Waiting for setup…"}</p>
      {setup.machine ? (
        <p>
          Enabled:{" "}
          {(["shell", "files", "computer"] as const)
            .filter((c) => setup.machine?.[c])
            .join(", ")}
        </p>
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

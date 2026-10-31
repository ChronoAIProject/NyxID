import { Cpu, HardDrive, MemoryStick, Monitor, Server } from "lucide-react";
import { MachineCapabilityChoices } from "@/components/shared/machine-create-fields";
import {
  assertRemoteMachineAdmission,
  useRemoteMachineAdmission,
} from "@/hooks/use-remote-machine-admission";
import { useEffect, useRef, useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { Link } from "@tanstack/react-router";
import {
  useAppForm,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  useRemoteMachineEntitlement,
  useRemoteMachines,
  useRemoteMachine,
  useRemoteMachineQuote,
  useCreateRemoteMachine,
} from "@/hooks/use-machines";
import { useBillingWallet } from "@/hooks/use-billing";
import { useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { ApiError } from "@/lib/api-client";
import { exactCredits, formatExactCredits } from "@/lib/credits";
import {
  getRemoteMachineIntent,
  readRemoteMachineIntent,
  clearRemoteMachineIntent,
} from "@/lib/remote-machine-intent";
import {
  REMOTE_MACHINE_PROFILE,
  SHARED_LEGACY_WARNING,
  remoteMachineCreateRequestSchema,
  remoteMachineHostedConfigurationSchema,
  remoteMachinePaidQuoteSchema,
  isRemoteMachinePaidQuote,
} from "@/schemas/machines";
import { RemoteMachinePlatformReadiness } from "./remote-machine-platform-readiness";
import { RemoteMachineCreatePolicies } from "./remote-machine-create-policies";
import {
  RemoteMachineBillingPreflight,
  RemoteMachineQuota,
} from "./remote-machine-preflight";

const formSchema = remoteMachineHostedConfigurationSchema.extend({
  separationAcknowledged: z
    .boolean()
    .refine(Boolean, "Review and accept the selected Agent separation policy."),
  acknowledged: z
    .boolean()
    .refine(
      Boolean,
      "Review and accept the displayed profile, billing and data policies.",
    ),
});

type Props = {
  readonly onCreated: (
    machineId: string,
    operationId: string,
    specialistId: string | null,
  ) => void;
  readonly initialFreeSlot?: boolean;
  readonly allowPaidCreation?: boolean;
};

/** The single hosted creation form, mounted by the fourth machine setup option. */
export function RemoteMachineCreate(props: Props) {
  const ownerId = useAuthStore((state) => state.user?.id);
  const enabled = useRemoteMachineAdmission();
  if (!enabled)
    return (
      <p role="status">Remote Machines are not enabled for this account.</p>
    );
  return <HostedCreateForm key={ownerId} {...props} />;
}

function HostedCreateForm({
  onCreated,
  initialFreeSlot = true,
  allowPaidCreation = true,
}: Props) {
  const ownerId = useAuthStore((state) => state.user?.id);
  const entitlement = useRemoteMachineEntitlement();
  const occupiedMachine = useRemoteMachine(
    !entitlement.error
      ? (entitlement.data?.machine_id ?? undefined)
      : undefined,
  );
  const machines = useRemoteMachines(false);
  const quote = useRemoteMachineQuote();
  const create = useCreateRemoteMachine();
  const wallet = useBillingWallet();
  const agents = useNyxBotAgents();
  const contextsEnabled = useFeature(FEATURE_FLAG.MACHINE_CONTEXTS);
  const [stored] = useState(() => {
    try {
      return {
        intent: ownerId ? readRemoteMachineIntent(ownerId, "create") : null,
        scope: "create",
        unreadable: false,
      };
    } catch {
      return { intent: null, scope: "create", unreadable: true };
    }
  });
  const [saved, setSaved] = useState(stored.intent);
  const [observedIntent, setObservedIntent] = useState<string>();
  const [checking, setChecking] = useState(false);
  const [existingMachineId, setExistingMachineId] = useState<string>();
  const inFlight = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const intentIdentity = saved ? JSON.stringify(saved) : undefined;
  const [unreadable, setUnreadable] = useState(stored.unreadable);
  const [freeSlot, setFreeSlot] = useState(
    stored.intent?.body.quote_id
      ? false
      : (stored.intent?.body.free_slot ??
          (allowPaidCreation ? initialFreeSlot : true)),
  );
  const paidCreationBlocked = !allowPaidCreation && !freeSlot;
  const [error, setError] = useState<string>();
  const [conflict, setConflict] = useState(false);
  const [now, setNow] = useState(Date.now);
  const form = useAppForm<
    z.input<typeof formSchema>,
    unknown,
    z.output<typeof formSchema>
  >({
    resolver: zodResolver(formSchema),
    defaultValues: {
      ...remoteMachineHostedConfigurationSchema.parse(
        stored.intent?.body ?? { separation_choice: "automatic" },
      ),
      acknowledged: false,
      separationAcknowledged: false,
    },
  });
  const requestQuote = quote.mutate;
  useEffect(() => {
    if (paidCreationBlocked) return;
    requestQuote({
      free_slot: freeSlot,
      profile_version: REMOTE_MACHINE_PROFILE,
    });
  }, [requestQuote, freeSlot, paidCreationBlocked]);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const preflight = !quote.error ? quote.data : undefined;
  const policy = preflight?.policy;
  const paid = remoteMachinePaidQuoteSchema.safeParse(preflight);
  const offer =
    !freeSlot &&
    paid.success &&
    (!saved || saved.body.quote_id === paid.data.quote_id)
      ? paid.data
      : undefined;
  const expired = Boolean(
    preflight && Date.parse(preflight.valid_until) <= now,
  );
  const platform = !entitlement.error
    ? (preflight?.platform ?? entitlement.data?.platform)
    : undefined;
  const quota =
    preflight?.quota ??
    (!entitlement.error ? entitlement.data?.quota : undefined);
  const freeSlotStatus = entitlement.error
    ? "Unavailable"
    : entitlement.isPending || !entitlement.data
      ? "Checking…"
      : !entitlement.data.eligible
        ? "Ineligible"
        : entitlement.data.free_slot_available
          ? "Available"
          : "Occupied";
  const canCreate = Boolean(
    ownerId &&
    !paidCreationBlocked &&
    !unreadable &&
    !conflict &&
    !create.isPending &&
    !checking &&
    !existingMachineId &&
    (saved
      ? !entitlement.error &&
        !machines.error &&
        !entitlement.isFetching &&
        !machines.isFetching &&
        observedIntent === intentIdentity
      : !quote.isPending &&
        (freeSlot
          ? !quote.error &&
            !entitlement.error &&
            entitlement.data?.eligible &&
            entitlement.data.free_slot_available &&
            platform?.configuration_ready === true &&
            preflight &&
            preflight.free_slot_requested !== false &&
            !expired &&
            policy?.policy_version &&
            policy.data_policy_version
          : isRemoteMachinePaidQuote(preflight, now))),
  );
  const locked = Boolean(saved) || unreadable || create.isPending;
  const capabilities = form.watch("capabilities") ?? [];
  const options =
    agents.data?.agents.filter(
      (agent) =>
        agent.kind !== "nyxbot" &&
        agent.status !== "destroyed" &&
        agent.owner_kind !== "org" &&
        (!agent.owner_id || agent.owner_id === ownerId),
    ) ?? [];
  const selectedSpecialist = form.watch("specialist_id");
  const separationChoice = form.watch("separation_choice");
  const requiredBlocked =
    !saved && separationChoice === "require_separated" && !contextsEnabled;
  const separationPolicy =
    saved?.body.separation_policy_version ?? policy?.separation_policy_version;
  useEffect(() => {
    form.setValue("separationAcknowledged", false, {
      shouldDirty: false,
      shouldTouch: false,
      shouldValidate: false,
    });
  }, [separationPolicy, form]);
  const available =
    !wallet.error && wallet.data && wallet.data.owner_id === ownerId
      ? exactCredits(
          wallet.data.available,
          wallet.data.available_credits,
          "whole",
        )
      : null;

  async function submit(values: z.output<typeof formSchema>) {
    if (
      inFlight.current ||
      !canCreate ||
      requiredBlocked ||
      !ownerId ||
      useAuthStore.getState().user?.id !== ownerId
    )
      return;
    setError(undefined);
    try {
      assertRemoteMachineAdmission(ownerId);
      if (
        saved &&
        JSON.stringify(readRemoteMachineIntent(ownerId, stored.scope)) !==
          intentIdentity
      ) {
        setObservedIntent(undefined);
        return;
      }
      inFlight.current = true;
      setObservedIntent(undefined);
      const intent =
        saved ??
        getRemoteMachineIntent(ownerId, stored.scope, {
          ...remoteMachineHostedConfigurationSchema.parse(values),
          separation_policy_version: policy!.separation_policy_version,
          acknowledge_separation: values.separationAcknowledged,
          free_slot: freeSlot,
          profile_version: preflight!.profile.profile_version,
          policy_version: policy!.policy_version,
          data_policy_version: policy!.data_policy_version,
          acknowledge_shared_environment: true,
          ...(!freeSlot && offer
            ? { quote_id: offer.quote_id, accept_account_balance: true }
            : {}),
        });
      setSaved(intent);
      const response = await create.mutateAsync({
        body: remoteMachineCreateRequestSchema.parse(intent.body),
        idempotencyKey: intent.idempotencyKey,
      });
      if (useAuthStore.getState().user?.id !== ownerId) return;
      clearRemoteMachineIntent(ownerId, stored.scope, intent.idempotencyKey);
      onCreated(
        response.machine.node_id,
        response.operation.id,
        intent.body.specialist_id ?? null,
      );
    } catch (cause) {
      if (useAuthStore.getState().user?.id !== ownerId) return;
      setConflict(
        cause instanceof ApiError &&
          [400, 402, 403, 404, 409, 410, 422].includes(cause.status),
      );
      setError(
        cause instanceof Error
          ? cause.message
          : "The result is unknown. Retry the saved request to recover it.",
      );
    } finally {
      inFlight.current = false;
    }
  }
  async function checkStatus() {
    if (!ownerId || !saved || inFlight.current || create.isPending) return;
    inFlight.current = true;
    setChecking(true);
    setObservedIntent(undefined);
    setExistingMachineId(undefined);
    setError(undefined);
    try {
      const [current, listed] = await Promise.all([
        entitlement.refetch(),
        machines.refetch(),
      ]);
      if (
        !mounted.current ||
        useAuthStore.getState().user?.id !== ownerId ||
        JSON.stringify(readRemoteMachineIntent(ownerId, stored.scope)) !==
          intentIdentity
      )
        return;
      if (
        current.error ||
        !current.data ||
        listed.error ||
        !listed.data?.complete
      )
        throw new Error(
          "Status could not be confirmed. Check status again before retrying the saved request.",
        );
      // A known occupied personal slot is recovered through its existing detail
      // route, which reads the Machine and its active operation. Never infer a
      // saved request's identity from a Machine name or change its body/key.
      const existing = saved.body.free_slot && current.data.machine_id;
      if (existing) {
        if (
          !listed.data.machines.some((machine) => machine.node_id === existing)
        )
          throw new Error(
            "Status could not be confirmed. Check status again before retrying the saved request.",
          );
        setExistingMachineId(existing);
      } else {
        setObservedIntent(intentIdentity);
      }
    } catch (cause) {
      if (mounted.current && useAuthStore.getState().user?.id === ownerId)
        setError(
          cause instanceof Error
            ? cause.message
            : "Status could not be confirmed. Check status again before retrying the saved request.",
        );
    } finally {
      inFlight.current = false;
      if (mounted.current) setChecking(false);
    }
  }
  function refresh() {
    if (paidCreationBlocked) return;
    form.setValue("acknowledged", false);
    form.setValue("separationAcknowledged", false);
    void entitlement.refetch();
    requestQuote({
      free_slot: freeSlot,
      profile_version: REMOTE_MACHINE_PROFILE,
    });
  }
  function reviewAgain() {
    if (!ownerId || create.isPending) return;
    try {
      clearRemoteMachineIntent(ownerId, stored.scope);
      setSaved(null);
      setObservedIntent(undefined);
      setExistingMachineId(undefined);
      setUnreadable(false);
      setConflict(false);
      setError(undefined);
      refresh();
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Browser storage is unavailable.",
      );
    }
  }
  if (paidCreationBlocked) {
    return (
      <section className="space-y-3 rounded-xl border border-border bg-card p-4 text-12">
        <p role="status">
          Paid creation is temporarily unavailable on this page. Your saved
          request has been kept. Check your Machines for its result.
        </p>
        <Button asChild variant="link">
          <Link to="/assistant/machines">View machines</Link>
        </Button>
      </section>
    );
  }
  const slotCard = (
    <section
      aria-label="Personal free slot"
      className="space-y-3 rounded-xl border border-border bg-card p-4 text-12"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="flex items-center gap-2 text-15 font-semibold">
          <Server className="size-4 text-text-tertiary" />
          {freeSlot ? "Personal free slot" : "Paid account-balance runtime"}
        </h3>
        {freeSlot ? (
          <Badge
            variant={
              freeSlotStatus === "Available"
                ? "success"
                : freeSlotStatus === "Checking…"
                  ? "secondary"
                  : "warning"
            }
          >
            {freeSlotStatus}
          </Badge>
        ) : null}
      </div>
      {freeSlot ? (
        <>
          {preflight ? (
            <div
              aria-label="Machine specifications"
              className="grid grid-cols-2 gap-3 sm:grid-cols-4"
            >
              {[
                {
                  icon: Cpu,
                  value: `${preflight.profile.cpu_millis / 1000} vCPU`,
                  label: "Compute",
                },
                {
                  icon: MemoryStick,
                  value: `${preflight.profile.memory_mib / 1024} GiB`,
                  label: "Memory",
                },
                {
                  icon: HardDrive,
                  value: `${preflight.profile.workspace_gib} GiB`,
                  label: "Workspace",
                },
                { icon: Monitor, value: "Linux", label: "System" },
              ].map(({ icon: Icon, value, label }) => (
                <div key={label} className="flex items-center gap-2.5">
                  <Icon className="size-4 shrink-0 text-text-tertiary" />
                  <div>
                    <p className="font-medium">{value}</p>
                    <p className="text-11 text-text-tertiary">{label}</p>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <p>
              Machine specification is unavailable until preflight succeeds.
            </p>
          )}
          <p className="border-t border-border/50 pt-3 text-muted-foreground">
            Machine runtime is free. Model and external API usage may be billed
            separately.
          </p>
        </>
      ) : null}
    </section>
  );
  if (
    freeSlot &&
    !saved &&
    !entitlement.error &&
    entitlement.data?.eligible &&
    !entitlement.data.free_slot_available
  )
    return (
      <div className="space-y-4">
        {slotCard}
        <section className="space-y-3 rounded-lg border border-border p-4 text-12">
          <p>
            Your free slot is already in use. Paused machines and pending
            deletions continue to occupy it.
          </p>
          {occupiedMachine.data && !occupiedMachine.error && (
            <p className="font-medium">
              {occupiedMachine.data.name} ·{" "}
              {occupiedMachine.data.resource_state.replaceAll("_", " ")}
            </p>
          )}
          {entitlement.data.machine_id && (
            <Button asChild variant="primary">
              <Link
                to="/assistant/machines"
                search={{ machine: entitlement.data.machine_id }}
              >
                View machine
              </Link>
            </Button>
          )}
        </section>
      </div>
    );
  return (
    <Form {...form}>
      <form
        aria-label="NyxID hosted machine configuration"
        className="space-y-4"
        onSubmit={form.handleSubmit(submit)}
      >
        {slotCard}
        <div className="space-y-2 text-12">
          <p className="font-medium">Machine owner</p>
          <Select value="personal" disabled>
            <SelectTrigger aria-label="Machine owner">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="personal">Personal account</SelectItem>
            </SelectContent>
          </Select>
        </div>
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
        <FormField
          control={form.control}
          name="separation_choice"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Agent separation</FormLabel>
              <Select
                value={field.value}
                disabled={locked}
                onValueChange={(value) => {
                  field.onChange(value);
                  form.setValue("separationAcknowledged", false);
                }}
              >
                <FormControl>
                  <SelectTrigger aria-label="Agent separation">
                    <SelectValue />
                  </SelectTrigger>
                </FormControl>
                <SelectContent>
                  <SelectItem value="automatic">
                    Automatic — require confirmed supported separation
                  </SelectItem>
                  <SelectItem value="require_separated">
                    Require separated workspaces and browsers
                  </SelectItem>
                  <SelectItem value="shared_legacy">
                    Explicit shared legacy access
                  </SelectItem>
                </SelectContent>
              </Select>
              <FormMessage />
            </FormItem>
          )}
        />
        <section
          aria-label="Agent separation policy"
          className="space-y-2 text-12"
        >
          <p>
            {separationChoice === "automatic"
              ? "Automatic waits for confirmed separation support and account permission. If unavailable, the owner must resolve the choice; it never silently selects shared access."
              : separationChoice === "require_separated"
                ? "Require separated refuses Agent execution until complete separation support is confirmed. It never falls back to shared access."
                : "Shared legacy records your explicit shared environment choice for new Agent assignments."}
          </p>
          {separationChoice === "shared_legacy" ? (
            <p>{policy?.separation_warning ?? SHARED_LEGACY_WARNING}</p>
          ) : null}
          {!contextsEnabled ? (
            <p role="status">
              Context setup is not enabled for this account. Automatic remains
              pending owner resolution when separation cannot be authorized;
              Require separated cannot be submitted.
            </p>
          ) : null}
          <FormField
            control={form.control}
            name="separationAcknowledged"
            render={({ field }) => (
              <FormItem className="flex items-start gap-2">
                <FormControl>
                  <Checkbox
                    checked={field.value}
                    onCheckedChange={(value) =>
                      form.setValue("separationAcknowledged", value === true)
                    }
                    disabled={create.isPending || (!saved && !separationPolicy)}
                  />
                </FormControl>
                <div>
                  <FormLabel>
                    I accept the selected Agent separation default and its
                    displayed policy, including the full warning when choosing
                    Shared legacy.
                  </FormLabel>
                  <FormMessage />
                </div>
              </FormItem>
            )}
          />
        </section>
        <MachineCapabilityChoices
          value={capabilities}
          disabled={locked}
          onChange={(next) => form.setValue("capabilities", next)}
        />
        <FormField
          control={form.control}
          name="specialist_id"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Select a specialist for this machine</FormLabel>
              <Select
                value={field.value ?? "none"}
                disabled={locked || agents.isPending || Boolean(agents.error)}
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
                  <SelectItem value="none">No specialist assignment</SelectItem>
                  {selectedSpecialist &&
                  !options.some((agent) => agent.id === selectedSpecialist) ? (
                    <SelectItem value={selectedSpecialist}>
                      Previously selected specialist
                    </SelectItem>
                  ) : null}
                  {options.map((agent) => (
                    <SelectItem key={agent.id} value={agent.id}>
                      {agent.display_name ?? agent.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FormMessage />
            </FormItem>
          )}
        />
        {agents.error ? (
          <ErrorBanner
            message="Specialists could not be loaded. You can create without assignment or retry loading."
            onRetry={() => void agents.refetch()}
          />
        ) : null}
        <p className="text-12 text-muted-foreground">
          Enabling a capability does not grant access to a client. Specialist
          assignment creates membership only, with all tools and saved logins
          off regardless of editing flags. After creation, Access opens for the
          selected Specialist so you can review its mode and grant specific
          tools. External OAuth and Agent Key tools remain limited to authorized
          Commands and Files.
        </p>
        <details className="rounded-lg border border-border p-4 text-12">
          <summary className="cursor-pointer font-medium">
            How Agent access works
          </summary>
          <div className="mt-3 space-y-3 text-muted-foreground">
            <p>
              Agents can run code and access this Machine’s workspace. Prompt
              injection is possible. Shared legacy agents and external clients
              share their configured OS user and workspace; file roots do not
              isolate shell commands.
            </p>
            <p>
              Separate workspaces and browser profiles share the host kernel.
              The machine default applies to new Agent assignments and does not
              grant tools or change existing permissions. External clients
              retain their explicit shared Commands/Files grants.
            </p>
            <p>
              Actual hosted support is verified after startup. Machine Ready
              does not mean an Agent is available. A failed Agent probe does not
              pause the machine, stop runtime charges, refund fees or delete
              data. Use Access to resolve authorization, or the existing Pause
              machine and Delete machine actions.
            </p>
            <p>
              Separation policy:{" "}
              {separationPolicy ?? "Unavailable — refresh preflight"}
            </p>
          </div>
        </details>
        <section
          aria-label="Remote machine preflight"
          className="space-y-3 rounded-xl border border-border bg-card p-4 text-12"
        >
          <h3 className="text-13 font-semibold">Runtime and data policy</h3>
          {allowPaidCreation ? (
            <Button
              type="button"
              variant="secondary"
              disabled={locked}
              onClick={() => {
                setFreeSlot(!freeSlot);
                form.setValue("acknowledged", false);
              }}
            >
              {freeSlot ? "Review paid creation" : "Review free slot"}
            </Button>
          ) : null}
          {saved ? (
            <p>
              A previously submitted request is saved for this account.
              Configuration is locked. Check status before explicitly retrying
              the same request, original policies and quote, even if fresh
              preflight is unavailable. It does not create a replacement.
            </p>
          ) : null}
          {saved?.body.quote_id ? (
            <p>Original quote: {saved.body.quote_id}</p>
          ) : null}
          {entitlement.error ? (
            <ErrorBanner
              message={`Eligibility is unavailable. ${entitlement.error.message}`}
              onRetry={refresh}
            />
          ) : !entitlement.isPending && entitlement.data ? (
            <>
              {!freeSlot ? (
                <Badge
                  variant={entitlement.data.eligible ? "success" : "warning"}
                >
                  {entitlement.data.eligible ? "Eligible" : "Ineligible"}
                </Badge>
              ) : null}
              {!entitlement.data.eligible ? (
                <p>This account is not eligible for a personal free slot.</p>
              ) : null}
              {entitlement.data.reason ? (
                <p>{entitlement.data.reason.replaceAll("_", " ")}</p>
              ) : null}
              {entitlement.data.machine_id ? (
                <Button asChild variant="link">
                  <Link
                    to="/assistant/machines"
                    search={{ machine: entitlement.data.machine_id }}
                  >
                    View machine
                  </Link>
                </Button>
              ) : null}
            </>
          ) : null}
          {quote.error ? (
            <ErrorBanner
              message={`Preflight is unavailable. ${quote.error.message}`}
              onRetry={refresh}
            />
          ) : null}
          {quote.isPending ? <p role="status">Refreshing preflight…</p> : null}
          {!freeSlot ? (
            <>
              {saved && !offer ? (
                <p>
                  The original quoted price remains bound to the saved request.
                  Its runtime estimate is unavailable here.
                </p>
              ) : offer ? (
                <>
                  <p>
                    Offered rate:{" "}
                    <strong>
                      {offer.credits_per_second} Credits per cumulative whole
                      running second
                    </strong>
                    .
                  </p>
                  <p>Rate version: {offer.rate_version}</p>
                  <p>
                    Quote expires:{" "}
                    {new Date(offer.valid_until).toLocaleString()}. Quotes are
                    valid for 15 minutes.
                  </p>
                </>
              ) : (
                <p role="alert">
                  A synchronized, priced runtime quote is unavailable. Review a
                  fresh quote before starting.
                </p>
              )}
              <RemoteMachineBillingPreflight
                preflight={
                  offer?.billing_preflight ??
                  (!saved ? entitlement.data?.billing_preflight : undefined)
                }
                quoteProfile={
                  offer?.profile.profile_version ?? REMOTE_MACHINE_PROFILE
                }
                quoteRate={offer?.rate_version}
                quotePrice={offer?.credits_per_second}
                now={now}
              />
              <p>
                Charging draws continuously from the shared account balance with
                no per-instance spending cap. A ready machine continues billing
                while idle; there is no automatic idle pause. Cumulative whole
                running seconds are charged at the accepted rate; a final
                fractional second is free. Downstream service usage is charged
                separately.
              </p>
            </>
          ) : null}
          {freeSlot &&
          !entitlement.error &&
          platform?.configuration_ready === false ? (
            <ErrorBanner
              message={`Machine setup is unavailable. ${platform.blockers.length ? platform.blockers.map((blocker) => blocker.replaceAll("_", " ")).join("; ") : "Platform configuration is not ready."}`}
              onRetry={refresh}
            />
          ) : null}
          {freeSlot && platform?.configuration_ready === true ? (
            <p className="text-muted-foreground">
              {quota?.status === "insufficient"
                ? "Available capacity may be insufficient. Creation will check current capacity."
                : "Capacity will be checked during creation."}
            </p>
          ) : null}
          {expired ? (
            <p role="alert">
              This preflight has expired. Refresh before making a new request.
            </p>
          ) : null}
          {allowPaidCreation ? (
            <>
              <p>
                Account wallet available:{" "}
                {available === null
                  ? "Unavailable"
                  : `${formatExactCredits(available, 12)} Credits`}
                . Free runtime does not require a funded wallet. Downstream
                services use their own billing and authorization.
              </p>
              <p>
                Paid creation requires a live synchronized quote and explicit
                account-balance acceptance. An occupied free slot never switches
                this request to paid. Paid running Machines share account funds
                and have no per-instance spending cap; adding funds never
                resumes a paused Machine automatically.
              </p>
            </>
          ) : null}
          <RemoteMachineCreatePolicies
            profile={preflight?.profile}
            policyVersion={saved?.body.policy_version ?? policy?.policy_version}
            dataPolicyVersion={
              saved?.body.data_policy_version ?? policy?.data_policy_version
            }
            expanded={!freeSlot}
          />
          <details open={!freeSlot} className="border-t border-border pt-3">
            <summary className="cursor-pointer font-medium focus-visible:outline-primary">
              Technical details
            </summary>
            <div className="mt-3 space-y-3 text-muted-foreground">
              <RemoteMachinePlatformReadiness platform={platform} />
              <RemoteMachineQuota quota={quota} />
              {quota ? (
                <p>
                  Quota observed:{" "}
                  {quota.observed_at
                    ? new Date(quota.observed_at).toLocaleString()
                    : "Unavailable"}
                  . Valid until:{" "}
                  {quota.valid_until
                    ? new Date(quota.valid_until).toLocaleString()
                    : "Unavailable"}
                  .
                </p>
              ) : null}
              {saved?.body.profile_version ? (
                <p>Original profile: {saved.body.profile_version}</p>
              ) : null}
            </div>
          </details>
        </section>
        <FormField
          control={form.control}
          name="acknowledged"
          render={({ field }) => (
            <FormItem className="flex items-start gap-3">
              <FormControl>
                <Checkbox
                  checked={field.value}
                  onCheckedChange={(value) =>
                    form.setValue("acknowledged", value === true)
                  }
                  disabled={
                    create.isPending ||
                    (!saved &&
                      !freeSlot &&
                      (quote.isPending ||
                        !isRemoteMachinePaidQuote(preflight, now)))
                  }
                />
              </FormControl>
              <div>
                <FormLabel>
                  {freeSlot
                    ? "I accept the shared environment, fixed specification, and data policies."
                    : "I accept this exact rate, continuous account-balance charging with no per-instance cap, and the displayed profile, shared environment, and data policy versions."}
                </FormLabel>
                <FormMessage />
              </div>
            </FormItem>
          )}
        />
        {existingMachineId ? (
          <p role="status">
            Continue with the existing machine to view its current operation.
          </p>
        ) : null}
        {existingMachineId &&
        existingMachineId !== entitlement.data?.machine_id ? (
          <Button asChild variant="link">
            <Link
              to="/assistant/machines"
              search={{ machine: existingMachineId }}
            >
              View machine
            </Link>
          </Button>
        ) : null}
        {error ? <ErrorBanner message={error} /> : null}
        {unreadable ? (
          <ErrorBanner message="The saved request cannot be read. Review and reset it before continuing." />
        ) : null}
        {conflict || unreadable ? (
          <Button type="button" variant="secondary" onClick={reviewAgain}>
            Discard saved request and review again
          </Button>
        ) : null}
        {create.isPending && (
          <p role="status" className="text-12 text-muted-foreground">
            Submitting your request. Waiting for the platform to confirm
            acceptance.
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          {saved && !conflict ? (
            <Button
              type="button"
              variant="secondary"
              onClick={() => void checkStatus()}
              disabled={checking || create.isPending}
            >
              {checking ? "Checking status…" : "Check status"}
            </Button>
          ) : null}
          <Button
            type="button"
            variant="secondary"
            onClick={refresh}
            disabled={create.isPending}
          >
            Refresh preflight
          </Button>
          <Button
            type="submit"
            variant="primary"
            isLoading={create.isPending}
            disabled={
              !canCreate ||
              requiredBlocked ||
              !form.watch("separationAcknowledged") ||
              !form.watch("acknowledged") ||
              !capabilities.length ||
              !form.watch("name")
            }
          >
            {saved
              ? "Retry saved create request"
              : freeSlot
                ? "Create free remote machine"
                : "Accept quote and create"}
          </Button>
        </div>
      </form>
    </Form>
  );
}

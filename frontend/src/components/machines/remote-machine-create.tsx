import {
  assertRemoteMachineAdmission,
  useRemoteMachineAdmission,
} from "@/hooks/use-remote-machine-admission";
import { useEffect, useState } from "react";
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
  useRemoteMachineQuote,
  useCreateRemoteMachine,
} from "@/hooks/use-machines";
import { useBillingWallet } from "@/hooks/use-billing";
import { useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
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
  remoteMachineCreateRequestSchema,
  remoteMachineHostedConfigurationSchema,
  remoteMachinePaidQuoteSchema,
  isRemoteMachinePaidQuote,
} from "@/schemas/machines";
import { RemoteMachinePlatformReadiness } from "./remote-machine-platform-readiness";
import {
  RemoteMachineBillingPreflight,
  RemoteMachineQuota,
} from "./remote-machine-preflight";

const formSchema = remoteMachineHostedConfigurationSchema.extend({
  acknowledged: z
    .boolean()
    .refine(
      Boolean,
      "Review and accept the displayed profile, billing and data policies.",
    ),
});
const labels = {
  shell: "Commands — run code and use connected services",
  files: "Files — read, edit and move files in workspace roots",
  browser: "Browser — use the managed browser",
  computer: "Computer — operate the desktop and browser",
};
type Props = {
  readonly onCreated: (machineId: string, operationId: string) => void;
  readonly initialFreeSlot?: boolean;
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

function HostedCreateForm({ onCreated, initialFreeSlot = true }: Props) {
  const user = useAuthStore((state) => state.user);
  const ownerId = user?.id;
  const entitlement = useRemoteMachineEntitlement();
  const quote = useRemoteMachineQuote();
  const create = useCreateRemoteMachine();
  const wallet = useBillingWallet();
  const agents = useNyxBotAgents();
  const [stored] = useState(() => {
    try {
      const free = ownerId ? readRemoteMachineIntent(ownerId, "create") : null;
      const paid =
        !free && ownerId
          ? readRemoteMachineIntent(ownerId, "create-paid")
          : null;
      return {
        intent: free ?? paid,
        scope: paid ? "create-paid" : "create",
        unreadable: false,
      };
    } catch {
      return { intent: null, scope: "create", unreadable: true };
    }
  });
  const [saved, setSaved] = useState(stored.intent);
  const [unreadable, setUnreadable] = useState(stored.unreadable);
  const [freeSlot, setFreeSlot] = useState(
    stored.intent?.body.free_slot ?? initialFreeSlot,
  );
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
        stored.intent?.body ?? {},
      ),
      acknowledged: false,
    },
  });
  const requestQuote = quote.mutate;
  useEffect(() => {
    requestQuote({
      free_slot: freeSlot,
      profile_version: REMOTE_MACHINE_PROFILE,
    });
  }, [requestQuote, freeSlot]);
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
  const canCreate = Boolean(
    ownerId &&
    !unreadable &&
    !conflict &&
    !create.isPending &&
    (saved ||
      (!quote.isPending &&
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
          : isRemoteMachinePaidQuote(preflight, now)))),
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
  const available =
    !wallet.error && wallet.data && wallet.data.owner_id === ownerId
      ? exactCredits(
          wallet.data.available,
          wallet.data.available_credits,
          "whole",
        )
      : null;

  async function submit(values: z.output<typeof formSchema>) {
    if (!canCreate || !ownerId || useAuthStore.getState().user?.id !== ownerId)
      return;
    setError(undefined);
    try {
      assertRemoteMachineAdmission(ownerId);
      const intent =
        saved ??
        getRemoteMachineIntent(ownerId, stored.scope, {
          ...remoteMachineHostedConfigurationSchema.parse(values),
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
      onCreated(response.machine.node_id, response.operation.id);
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
    }
  }
  function refresh() {
    form.setValue("acknowledged", false);
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
      // A malformed legacy paid-create intent must not be left behind on reset.
      if (unreadable) clearRemoteMachineIntent(ownerId, "create-paid");
      setSaved(null);
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
  return (
    <Form {...form}>
      <form
        aria-label="NyxID hosted machine configuration"
        className="space-y-4"
        onSubmit={form.handleSubmit(submit)}
      >
        <div className="space-y-1 text-12">
          <p className="font-medium">Machine owner</p>
          <p>Personal account{user?.email ? ` · ${user.email}` : ""}</p>
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
                  disabled={
                    locked ||
                    (capability === "computer" &&
                      !capabilities.includes("browser"))
                  }
                  onCheckedChange={(checked) =>
                    form.setValue(
                      "capabilities",
                      checked
                        ? [...capabilities, capability]
                        : capabilities.filter(
                            (item) =>
                              item !== capability &&
                              !(
                                capability === "browser" && item === "computer"
                              ),
                          ),
                    )
                  }
                />
                {labels[capability]}
              </label>
            ),
          )}
          {form.formState.errors.capabilities?.message ? (
            <p role="alert" className="text-12 text-destructive">
              {form.formState.errors.capabilities.message}
            </p>
          ) : null}
        </fieldset>
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
          assignment and permissions are separate; review any pending
          permissions in agent settings after creation. External OAuth and Agent
          Key tools remain limited to authorized Commands and Files.
        </p>
        <section
          aria-label="Remote machine preflight"
          className="space-y-3 rounded-xl border border-border bg-card p-4 text-12"
        >
          <h3 className="text-13 font-semibold">
            {freeSlot ? "Personal free slot" : "Paid account-balance runtime"}
          </h3>
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
          {saved ? (
            <p>
              A previously submitted request is saved for this account.
              Configuration is locked. Retrying recovers the same Machine
              request, original policies and quote even if fresh preflight is
              unavailable. It does not create a replacement.
            </p>
          ) : null}
          {saved?.body.quote_id ? (
            <p>Original quote: {saved.body.quote_id}</p>
          ) : null}
          {saved?.body.profile_version ? (
            <p>Original profile: {saved.body.profile_version}</p>
          ) : null}
          {entitlement.error ? (
            <ErrorBanner
              message={`Eligibility is unavailable. ${entitlement.error.message}`}
              onRetry={refresh}
            />
          ) : entitlement.data ? (
            <>
              <Badge
                variant={entitlement.data.eligible ? "success" : "warning"}
              >
                {entitlement.data.eligible ? "Eligible" : "Ineligible"}
              </Badge>
              <p>
                {entitlement.data.eligible
                  ? entitlement.data.free_slot_available
                    ? "Your personal free slot is available."
                    : "Your personal free slot is occupied."
                  : "This account is not eligible for a personal free slot."}
              </p>
              {entitlement.data.reason ? (
                <p>{entitlement.data.reason.replaceAll("_", " ")}</p>
              ) : null}
              {entitlement.data.machine_id ? (
                <Button asChild variant="link">
                  <Link
                    to="/assistant/machines"
                    search={{ machine: entitlement.data.machine_id }}
                  >
                    View the machine occupying this slot
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
          <RemoteMachinePlatformReadiness platform={platform} />
          <RemoteMachineQuota
            quota={
              preflight?.quota ??
              (!entitlement.error ? entitlement.data?.quota : undefined)
            }
          />
          {expired ? (
            <p role="alert">
              This preflight has expired. Refresh before making a new request.
            </p>
          ) : null}
          <p>
            {preflight ? (
              <>
                Profile: {preflight.profile.profile_version}. New Machine fixed
                specification: {preflight.profile.cpu_millis / 1000} vCPU ·{" "}
                {preflight.profile.memory_mib / 1024} GiB memory ·{" "}
                {preflight.profile.workspace_gib} GiB /workspace.{" "}
              </>
            ) : (
              "Machine specification is unavailable until preflight succeeds. "
            )}
            Memory includes the supervisor, browser, desktop and output cache.
            Workspace expansion and automatic upgrades are not available.
            Bandwidth and IOPS are not unlimited.
          </p>
          <p>
            Temporary storage is managed by the deployment environment, with no
            fixed capacity commitment, selectable size or extra storage charge.
            HOME, /tmp, logs and the writable layer remain temporary. Temporary
            data may be lost after pause or Pod recreation. Full disks can cause
            writes and jobs to fail, and disk pressure can evict the Pod.
          </p>
          <p>
            Each personal account has one free slot, with no entitlement expiry
            or monthly runtime limit. A paused Machine still occupies the slot
            and retains its workspace indefinitely until owner-confirmed
            deletion. Cleanup and accounting finalization must complete before
            the slot becomes available again.
          </p>
          <p>
            Account wallet available:{" "}
            {available === null
              ? "Unavailable"
              : `${formatExactCredits(available, 12)} Credits`}
            . Free runtime does not require a funded wallet. Downstream services
            use their own billing and authorization.
          </p>
          <p>
            Paid creation requires a live synchronized quote and explicit
            account-balance acceptance. An occupied free slot never switches
            this request to paid. Paid running Machines share account funds and
            have no per-instance spending cap; adding funds never resumes a
            paused Machine automatically.
          </p>
          <p>
            All clients share the OS and workspace. File roots do not isolate
            shell commands. No inbound ports, public IP, persistent environment
            variables, snapshots or restore. Only /workspace persists; HOME,
            /tmp, browser profiles and downloads are temporary unless safely
            saved in the workspace. Job environment variables are temporary.
          </p>
          <p>
            At most two jobs run concurrently. Default command timeout is 120
            seconds, maximum one hour. Each output cache is bounded to 1 MiB;
            the Machine cache is bounded to 32 MiB. Raw output remains only on
            the Node, lasts at most one hour after termination, and may
            disappear sooner.
          </p>
          <p>
            Job metadata remains for 30 days and terminal operation history for
            90 days. Deletion permanently removes the workspace, targets cleanup
            within 24 hours, and retains necessary billing, cleanup and
            idempotency evidence. Paused workspace storage is free.
          </p>
          {policy || saved ? (
            <p>
              Policy: {saved?.body.policy_version ?? policy?.policy_version}.
              Data policy:{" "}
              {saved?.body.data_policy_version ?? policy?.data_policy_version}.
            </p>
          ) : null}
        </section>
        <FormField
          control={form.control}
          name="acknowledged"
          render={({ field }) => (
            <FormItem className="flex items-start gap-3">
              <FormControl>
                <Checkbox
                  checked={field.value}
                  onCheckedChange={(value) => field.onChange(value === true)}
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
        {error ? <ErrorBanner message={error} /> : null}
        {unreadable ? (
          <ErrorBanner message="The saved request cannot be read. Review and reset it before continuing." />
        ) : null}
        {conflict || unreadable ? (
          <Button type="button" variant="secondary" onClick={reviewAgain}>
            Discard saved request and review again
          </Button>
        ) : null}
        <div className="flex flex-wrap gap-2">
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

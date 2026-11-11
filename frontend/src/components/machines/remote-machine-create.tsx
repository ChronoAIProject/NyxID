import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { Label } from "@/components/ui/label";
import { flushSync } from "react-dom";
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
  FormLabel,
} from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Switch } from "@/components/ui/switch";
import {
  GroupedList,
  GroupedRow,
  GroupedFooter,
  GroupedInput as Input,
  GroupedSelectTrigger as SelectTrigger,
} from "@/components/ui/grouped-list";
import {
  GroupedField as FormItem,
  GroupedMessage as FormMessage,
} from "@/components/ui/grouped-fields";
import { GroupedDialogActions } from "@/components/ui/grouped-form";
import {
  Select,
  SelectContent,
  SelectItem,
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
  readonly onCancel?: () => void;
  readonly initialFreeSlot?: boolean;
};

/** The single hosted creation form, mounted by the fourth machine setup option. */
export function RemoteMachineCreate(props: Props) {
  useUiTranslation();
  const ownerId = useAuthStore((state) => state.user?.id);
  const enabled = useRemoteMachineAdmission();
  if (!enabled)
    return (
      <p role="status">{translateUi("Remote Machines are not enabled for this account.")}</p>
    );
  return <HostedCreateForm key={ownerId} {...props} />;
}

function HostedCreateForm({ onCreated, onCancel, initialFreeSlot = true }: Props) {
  useUiTranslation();
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
    mode: "onChange",
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
    if (!form.formState.isValid || !canCreate || !ownerId || useAuthStore.getState().user?.id !== ownerId)
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
      flushSync(() => {
        setSaved(null);
        form.reset();
      });
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
        aria-label={translateUi("NyxID hosted machine configuration")}
        className="space-y-6"
        onSubmit={form.handleSubmit(submit)}
      >
        <GroupedList title={translateUi("Machine configuration")}>
          <GroupedRow
            title={translateUi("Machine owner")}
            description={translateUi("Personal account{{value1}}", { value1: user?.email ? ` · ${user.email}` : "" })}
          />
          <FormField
            control={form.control}
            name="name"
            render={({ field }) => (
              <FormItem>
                <FormLabel className="text-text-tertiary">{translateUi("Machine name")}</FormLabel>
                <FormControl>
                  <Input {...field} disabled={locked} autoComplete="off" />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
        </GroupedList>
        <GroupedList title={translateUi("Enable on this machine")}>
          <fieldset disabled={locked}>
            <Label asChild><legend className="sr-only">{translateUi("Enable on this machine")}</legend></Label>
            {(["shell", "files", "browser", "computer"] as const).map(
              (capability) => (
                <GroupedRow key={capability} asChild interactive>
                  <Label>
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
                  </Label>
                </GroupedRow>
              ),
            )}
            {form.formState.errors.capabilities?.message ? (
              <GroupedFooter>
                <p role="alert" className="text-destructive">
                  {form.formState.errors.capabilities.message}
                </p>
              </GroupedFooter>
            ) : null}
          </fieldset>
        </GroupedList>
        <GroupedList
          title={translateUi("Specialist access")}
          footer={translateUi("Enabling a capability does not grant access to a client. Specialist assignment and permissions are separate; review any pending permissions in agent settings after creation. External OAuth and Agent Key tools remain limited to authorized Commands and Files.")}
        >
          <FormField
            control={form.control}
            name="specialist_id"
            render={({ field }) => (
              <FormItem inline>
                <FormLabel>{translateUi("Select a specialist for this machine")}</FormLabel>
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
                    <SelectItem value="none">{translateUi("No specialist assignment")}</SelectItem>
                    {selectedSpecialist &&
                    !options.some((agent) => agent.id === selectedSpecialist) ? (
                      <SelectItem value={selectedSpecialist}>{translateUi("Previously selected specialist")}</SelectItem>
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
          <GroupedFooter>
            {agents.error ? (
              <ErrorBanner
                message={translateUi("Specialists could not be loaded. You can create without assignment or retry loading.")}
                onRetry={() => void agents.refetch()}
              />
            ) : null}
          </GroupedFooter>
        </GroupedList>
        <GroupedList
          title={
            freeSlot ? translateUi("Personal free slot") : translateUi("Paid account-balance runtime")
          }
          aria-label={translateUi("Remote machine preflight")}
          footer={
            <>
              {saved ? (
                <p>{translateUi("A previously submitted request is saved for this account. Configuration is locked. Retrying recovers the same Machine request, original policies and quote even if fresh preflight is unavailable. It does not create a replacement.")}</p>
              ) : null}
              {saved?.body.quote_id ? (
                <p>{translateUi("Original quote: ")}{saved.body.quote_id}</p>
              ) : null}
              {saved?.body.profile_version ? (
                <p>{translateUi("Original profile: ")}{saved.body.profile_version}</p>
              ) : null}
              {entitlement.error ? (
                <ErrorBanner
                  message={translateUi("Eligibility is unavailable. {{value1}}", { value1: entitlement.error.message })}
                  onRetry={refresh}
                />
              ) : entitlement.data ? (
                <>
                  <p>
                    {entitlement.data.eligible
                      ? entitlement.data.free_slot_available
                        ? translateUi("Your personal free slot is available.")
                        : translateUi("Your personal free slot is occupied.")
                      : translateUi("This account is not eligible for a personal free slot.")}
                  </p>
                  {entitlement.data.reason ? (
                    <p>{entitlement.data.reason.replaceAll("_", " ")}</p>
                  ) : null}
                  {entitlement.data.machine_id ? (
                    <Button asChild variant="link">
                      <Link
                        to="/assistant/machines"
                        search={{ machine: entitlement.data.machine_id }}
                      >{translateUi("View the machine occupying this slot")}</Link>
                    </Button>
                  ) : null}
                </>
              ) : null}
              {quote.error ? (
                <ErrorBanner
                  message={translateUi("Preflight is unavailable. {{value1}}", { value1: quote.error.message })}
                  onRetry={refresh}
                />
              ) : null}
              {quote.isPending ? (
                <p role="status">{translateUi("Refreshing preflight…")}</p>
              ) : null}
              {!freeSlot ? (
                <>
                  {saved && !offer ? (
                    <p>{translateUi("The original quoted price remains bound to the saved request. Its runtime estimate is unavailable here.")}</p>
                  ) : offer ? (
                    <>
                      <p>{translateUi("Offered rate:")}{" "}
                        <strong>
                          {offer.credits_per_second}{translateUi(" Credits per cumulative whole running second")}</strong>
                        .
                      </p>
                      <p>{translateUi("Rate version: ")}{offer.rate_version}</p>
                      <p>{translateUi("Quote expires:")}{" "}
                        {new Date(offer.valid_until).toLocaleString(getFormattingLocale())}. Quotes are
                        valid for 15 minutes.
                      </p>
                    </>
                  ) : (
                    <p role="alert">{translateUi("A synchronized, priced runtime quote is unavailable. Review a fresh quote before starting.")}</p>
                  )}
                  <RemoteMachineBillingPreflight grouped
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
                  <p>{translateUi("Charging draws continuously from the shared account balance with no per-instance spending cap. A ready machine continues billing while idle; there is no automatic idle pause. Cumulative whole running seconds are charged at the accepted rate; a final fractional second is free. Downstream service usage is charged separately.")}</p>
                </>
              ) : null}
              <RemoteMachinePlatformReadiness grouped platform={platform} />
              <RemoteMachineQuota grouped
                quota={
                  preflight?.quota ??
                  (!entitlement.error ? entitlement.data?.quota : undefined)
                }
              />
              {expired ? (
                <p role="alert">{translateUi("This preflight has expired. Refresh before making a new request.")}</p>
              ) : null}
              <p>
                {preflight ? (
                  <>{translateUi("Profile: ")}{preflight.profile.profile_version}. New Machine
                    fixed specification: {preflight.profile.cpu_millis / 1000}{" "}{translateUi("vCPU · ")}{preflight.profile.memory_mib / 1024}{translateUi(" GiB memory ·")}{" "}
                    {preflight.profile.workspace_gib}{translateUi(" GiB /workspace.")}{" "}
                  </>
                ) : (
                  translateUi("Machine specification is unavailable until preflight succeeds. ")
                )}{translateUi("Memory includes the supervisor, browser, desktop and output cache. Workspace expansion and automatic upgrades are not available. Bandwidth and IOPS are not unlimited.")}</p>
              <p>{translateUi("Temporary storage is managed by the deployment environment, with no fixed capacity commitment, selectable size or extra storage charge. HOME, /tmp, logs and the writable layer remain temporary. Temporary data may be lost after pause or Pod recreation. Full disks can cause writes and jobs to fail, and disk pressure can evict the Pod.")}</p>
              <p>{translateUi("Each personal account has one free slot, with no entitlement expiry or monthly runtime limit. A paused Machine still occupies the slot and retains its workspace indefinitely until owner-confirmed deletion. Cleanup and accounting finalization must complete before the slot becomes available again.")}</p>
              <p>{translateUi("Account wallet available:")}{" "}
                {available === null
                  ? translateUi("Unavailable")
                  : translateUi("{{value1}} Credits", { value1: formatExactCredits(available, 12) })}
                . Free runtime does not require a funded wallet. Downstream services
                use their own billing and authorization.
              </p>
              <p>{translateUi("Paid creation requires a live synchronized quote and explicit account-balance acceptance. An occupied free slot never switches this request to paid. Paid running Machines share account funds and have no per-instance spending cap; adding funds never resumes a paused Machine automatically.")}</p>
              <p>{translateUi("All clients share the OS and workspace. File roots do not isolate shell commands. No inbound ports, public IP, persistent environment variables, snapshots or restore. Only /workspace persists; HOME, /tmp, browser profiles and downloads are temporary unless safely saved in the workspace. Job environment variables are temporary.")}</p>
              <p>{translateUi("At most two jobs run concurrently. Default command timeout is 120 seconds, maximum one hour. Each output cache is bounded to 1 MiB; the Machine cache is bounded to 32 MiB. Raw output remains only on the Node, lasts at most one hour after termination, and may disappear sooner.")}</p>
              <p>{translateUi("Job metadata remains for 30 days and terminal operation history for 90 days. Deletion queues workspace cleanup, targets completion within 24 hours, and retains necessary billing, cleanup and idempotency evidence. Underlying storage follows its reclaim policy; physical storage release is not verified. Paused workspace storage is free.")}</p>
              {policy || saved ? (
                <p>{translateUi("Policy: ")}{saved?.body.policy_version ?? policy?.policy_version}.
                  Data policy:{" "}
                  {saved?.body.data_policy_version ?? policy?.data_policy_version}.
                </p>
              ) : null}
            </>
          }
        >
          {!entitlement.error && entitlement.data && (
            <GroupedRow title={translateUi("Eligibility")} accessory={entitlement.data.eligible ? "Eligible" : "Ineligible"} />
          )}
          <GroupedRow asChild interactive>
            <button
              type="button"
              disabled={locked}
              onClick={() => {
                setFreeSlot(!freeSlot);
                form.setValue("acknowledged", false);
              }}
            >
              {freeSlot ? translateUi("Review paid creation") : translateUi("Review free slot")}
            </button>
          </GroupedRow>
        </GroupedList>
        <GroupedList title={translateUi("Acceptance")}>
          <FormField
            control={form.control}
            name="acknowledged"
            render={({ field }) => (
              <FormItem inline className="items-start">
                <FormControl>
                  <Switch
                    className="order-last shrink-0"
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
                <div className="min-w-0 flex-1">
                  <FormLabel className="text-foreground">
                    {freeSlot
                      ? translateUi("I accept the shared environment, fixed specification, and data policies.")
                      : translateUi("I accept this exact rate, continuous account-balance charging with no per-instance cap, and the displayed profile, shared environment, and data policy versions.")}
                  </FormLabel>
                  <FormMessage />
                </div>
              </FormItem>
            )}
          />
          <GroupedFooter>
            {error ? (
              <p role="alert" className="text-destructive">
                {error}
              </p>
            ) : null}
            {unreadable ? (
              <ErrorBanner message={translateUi("The saved request cannot be read. Review and reset it before continuing.")} />
            ) : null}
          </GroupedFooter>
        </GroupedList>
        <GroupedList title={translateUi("Preflight actions")}>
          {conflict || unreadable ? (
            <GroupedRow asChild interactive>
              <button
                type="button"
                className="text-destructive"
                onClick={reviewAgain}
              >{translateUi("Discard saved request and review again")}</button>
            </GroupedRow>
          ) : null}
          <GroupedRow asChild interactive>
            <button type="button" onClick={refresh} disabled={create.isPending}>{translateUi("Refresh preflight")}</button>
          </GroupedRow>
        </GroupedList>
        <GroupedDialogActions
          intent="create"
          cancel={onCancel ?? (() => window.location.assign("/assistant/machines"))}
          label={translateUi("hosted machine configuration")}
          dirty={form.formState.isDirty}
          pending={create.isPending}
          disabled={
            !form.formState.isValid || !canCreate ||
            !form.watch("acknowledged") ||
            !capabilities.length ||
            !form.watch("name")
          }
          save={form.handleSubmit(submit)}
          discard={() => {
            form.reset();
            setError(undefined);
          }}
          saveLabel={
            saved ? translateUi("Retry saved create request") : freeSlot
              ? translateUi("Create free remote machine")
              : translateUi("Accept quote and create")
          }
        />
      </form>
    </Form>
  );
}

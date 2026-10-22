import {
  assertRemoteMachineAdmission,
  useRemoteMachineAdmission,
} from "@/hooks/use-remote-machine-admission";
import {
  RemoteMachineBillingPreflight,
  RemoteMachineQuota,
} from "./remote-machine-preflight";
import { RemoteMachinePlatformReadiness } from "./remote-machine-platform-readiness";
import { useEffect, useState } from "react";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useAppForm,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  useRemoteMachineQuote,
  useRemoteMachineEntitlement,
  useRemoteMachineAction,
} from "@/hooks/use-machines";
import { useAuthStore } from "@/stores/auth-store";
import { ApiError } from "@/lib/api-client";
import { useBillingWallet } from "@/hooks/use-billing";
import { exactCredits, formatExactCredits } from "@/lib/credits";
import {
  clearRemoteMachineIntent,
  getRemoteMachineIntent,
  readRemoteMachineIntent,
  type RemoteMachineIntent,
} from "@/lib/remote-machine-intent";
import {
  isRemoteMachinePaidQuote,
  remoteMachinePaidQuoteSchema,
  remoteMachineActionRequestSchema,
  type RemoteMachine,
} from "@/schemas/machines";

const acceptanceSchema = z.object({
  accepted: z
    .boolean()
    .refine(
      Boolean,
      "Accept the quoted rate, continuous account-balance charging, and profile and data policies.",
    ),
});
type Props = {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly machine: RemoteMachine;
  readonly onAccepted: (machineId: string, operationId: string) => void;
};

export function RemoteMachinePaidStart({
  open,
  onOpenChange,
  machine,
  onAccepted,
}: Props) {
  const ownerId = useAuthStore((state) => state.user?.id);
  const enabled = useRemoteMachineAdmission();
  const [review, setReview] = useState({
    open,
    ownerId: enabled ? ownerId : undefined,
  });
  if (review.open !== open) {
    setReview({ open, ownerId: enabled ? ownerId : undefined });
  } else if (review.ownerId && (!enabled || review.ownerId !== ownerId)) {
    setReview({ open, ownerId: undefined });
  }
  const eligible = enabled && review.ownerId === ownerId;
  return (
    <Dialog open={open && eligible} onOpenChange={onOpenChange}>
      {open && eligible ? (
        <PaidStartContent
          key={`${ownerId}:${machine.node_id}`}
          machine={machine}
          onAccepted={onAccepted}
          onClose={() => onOpenChange(false)}
        />
      ) : null}
    </Dialog>
  );
}

function PaidStartContent({
  machine,
  onAccepted,
  onClose,
}: Pick<Props, "onAccepted"> & {
  readonly machine: RemoteMachine;
  readonly onClose: () => void;
}) {
  const ownerId = useAuthStore((state) => state.user?.id);
  const scope = `${machine.node_id}:resume`;
  const quote = useRemoteMachineQuote();
  const entitlement = useRemoteMachineEntitlement(
    Boolean(quote.error),
    machine.profile.profile_version,
  );
  const action = useRemoteMachineAction();
  const wallet = useBillingWallet();
  const [saved, setSaved] = useState<RemoteMachineIntent | null>(() => {
    try {
      return ownerId ? readRemoteMachineIntent(ownerId, scope) : null;
    } catch {
      return null;
    }
  });
  const [unreadable, setUnreadable] = useState(() => {
    try {
      if (ownerId) readRemoteMachineIntent(ownerId, scope);
      return false;
    } catch {
      return true;
    }
  });
  const [error, setError] = useState<string>();
  const [conflict, setConflict] = useState(false);
  const [now, setNow] = useState(Date.now);
  const form = useAppForm<z.infer<typeof acceptanceSchema>>({
    resolver: zodResolver(acceptanceSchema),
    defaultValues: { accepted: Boolean(saved) },
  });
  const requestQuote = quote.mutate;
  useEffect(() => {
    requestQuote({
      free_slot: false,
      machine_id: machine.node_id,
      profile_version: machine.profile.profile_version,
    });
  }, [requestQuote, machine]);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const parsed = remoteMachinePaidQuoteSchema.safeParse(quote.data);
  const offer =
    !quote.error &&
    parsed.success &&
    parsed.data.profile.profile_version === machine.profile.profile_version &&
    (!saved || saved.body.quote_id === parsed.data.quote_id)
      ? parsed.data
      : undefined;
  const acceptable = Boolean(offer && isRemoteMachinePaidQuote(offer, now));
  const expired = Boolean(offer && Date.parse(offer.valid_until) <= now);
  const busy = action.isPending;
  const available =
    !wallet.error && wallet.data && ownerId && wallet.data.owner_id === ownerId
      ? exactCredits(
          wallet.data.available,
          wallet.data.available_credits,
          "whole",
        )
      : null;
  const permittedState =
    machine.resource_state === "paused" &&
    machine.free_slot !== true &&
    machine.billing_state !== "free";
  const canSubmit = Boolean(
    ownerId &&
    !unreadable &&
    !conflict &&
    !busy &&
    (saved || (acceptable && permittedState && !quote.isPending)),
  );

  async function submit() {
    if (!canSubmit || !ownerId || useAuthStore.getState().user?.id !== ownerId)
      return;
    setError(undefined);
    try {
      assertRemoteMachineAdmission(ownerId);
      let intent = saved;
      if (!intent) {
        if (!isRemoteMachinePaidQuote(quote.data) || quote.error) return;
        const policies = {
          profile_version: quote.data.profile.profile_version,
          policy_version: quote.data.policy.policy_version,
          data_policy_version: quote.data.policy.data_policy_version,
          acknowledge_shared_environment: true,
          quote_id: quote.data.quote_id,
          accept_account_balance: true,
        };
        intent = getRemoteMachineIntent(ownerId, scope, {
          ...policies,
          action: "resume",
          expected_revision: machine.revision,
        });
        setSaved(intent);
      }
      const response = await action.mutateAsync({
        machineId: machine.node_id,
        body: remoteMachineActionRequestSchema.parse(intent.body),
        idempotencyKey: intent.idempotencyKey,
      });
      const result = {
        machineId: machine.node_id,
        operationId: response.operation.id,
      };
      if (useAuthStore.getState().user?.id !== ownerId) return;
      clearRemoteMachineIntent(ownerId, scope, intent.idempotencyKey);
      onAccepted(result.machineId, result.operationId);
      onClose();
    } catch (cause) {
      if (useAuthStore.getState().user?.id !== ownerId) return;
      setConflict(
        cause instanceof ApiError &&
          [400, 402, 403, 404, 409, 410, 422].includes(cause.status),
      );
      setError(
        cause instanceof Error
          ? cause.message
          : "The response is unknown. Retry the saved request to recover its result.",
      );
    }
  }

  function reviewAgain() {
    if (!ownerId || busy) return;
    try {
      clearRemoteMachineIntent(ownerId, scope);
      setSaved(null);
      setUnreadable(false);
      setConflict(false);
      setError(undefined);
      form.reset({ accepted: false });
      requestQuote({
        free_slot: false,
        machine_id: machine.node_id,
        profile_version: machine.profile.profile_version,
      });
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Browser storage is unavailable.",
      );
    }
  }

  return (
    <DialogContent className="md:max-w-2xl">
      <DialogHeader>
        <DialogTitle>Review paid resume</DialogTitle>
        <DialogDescription>
          Review a server-issued runtime quote before explicitly starting
          continuous account-balance charging.
        </DialogDescription>
      </DialogHeader>
      <Form {...form}>
        <form className="space-y-4" onSubmit={form.handleSubmit(submit)}>
          <section
            aria-label="Runtime quote"
            className="space-y-2 rounded-xl border border-border bg-card p-4 text-12"
          >
            {saved ? (
              <>
                <p>
                  A previously submitted request is saved for this account.
                  Retrying recovers the same request, including after the
                  original quote expires; it does not accept a replacement
                  quote.
                </p>
                <p className="break-all">
                  Original quote: {saved.body.quote_id ?? "Unavailable"}
                </p>
                {saved.body.profile_version ? (
                  <p>Original profile: {saved.body.profile_version}</p>
                ) : null}
                <p>
                  Original policies: {saved.body.policy_version} /{" "}
                  {saved.body.data_policy_version}
                </p>
              </>
            ) : quote.isPending ? (
              <p role="status">Requesting a synchronized runtime quote…</p>
            ) : quote.error ? (
              <ErrorBanner
                message={
                  quote.error instanceof Error
                    ? quote.error.message
                    : "Paid runtime quoting is unavailable."
                }
                onRetry={reviewAgain}
              />
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
                <p className="break-all">Rate version: {offer.rate_version}</p>
                <p>
                  Quote expires: {new Date(offer.valid_until).toLocaleString()}.
                  Quotes are valid for 15 minutes.
                </p>
                {expired ? (
                  <p role="alert" className="text-destructive">
                    This quote has expired. Review a fresh quote before
                    starting.
                  </p>
                ) : null}
                <p>
                  Profile: {offer.profile.profile_version}. Fixed{" "}
                  {offer.profile.cpu_millis / 1000} vCPU,{" "}
                  {offer.profile.memory_mib / 1024} GiB memory,{" "}
                  {offer.profile.workspace_gib} GiB /workspace.
                </p>
                <p>
                  Temporary storage is managed by the deployment environment. No
                  fixed capacity commitment, selectable size or extra storage
                  charge. Temporary data may be lost after pause or Pod
                  recreation. Full disks can cause writes to fail, and disk
                  pressure can evict the Pod. Review Machine details for
                  recovery guidance.
                </p>
                <p>
                  Policy: {offer.policy.policy_version}. Data policy:{" "}
                  {offer.policy.data_policy_version}.
                </p>
              </>
            ) : (
              <p role="alert">
                A synchronized, priced runtime quote is unavailable. This server
                has not supplied an acceptable paid offer.
              </p>
            )}
            <RemoteMachinePlatformReadiness
              platform={offer?.platform ?? entitlement.data?.platform}
            />
            <RemoteMachineQuota
              quota={offer?.quota ?? entitlement.data?.quota}
            />
            <RemoteMachineBillingPreflight
              preflight={
                offer?.billing_preflight ??
                (quote.error && !saved
                  ? entitlement.data?.billing_preflight
                  : undefined)
              }
              quoteProfile={
                offer?.profile.profile_version ??
                machine.profile.profile_version
              }
              quoteRate={offer?.rate_version}
              quotePrice={offer?.credits_per_second}
              now={now}
            />
            <p>
              Charging draws continuously from the shared account balance with
              no per-instance spending cap. Cumulative whole running seconds are
              charged at the accepted rate; a final fractional second is free. A
              ready machine continues billing while idle; there is no automatic
              idle pause. Paused workspace storage is free. Downstream service
              usage is charged separately.
            </p>
            <p>
              Account wallet available:{" "}
              {available === null
                ? "Unavailable"
                : `${formatExactCredits(available, 12)} Credits`}
              . The server checks current eligibility, subscription, provider
              readiness, and finite funding when you submit.
            </p>
            {machine ? (
              <p>
                Resume requires a new accepted quote and resolution of the
                previous run’s pending liability. Adding funds never resumes
                this Machine automatically.
              </p>
            ) : null}
            <p>
              All clients share the OS and workspace. File roots do not isolate
              shell commands. There are no inbound ports, public IP, persistent
              environment variables, snapshots, or restore. Job environment
              variables are temporary.
            </p>
            <p>
              Job metadata is retained for 30 days and terminal operation
              history for 90 days. Output is live-only, retained at most one
              hour after termination, and may disappear sooner. Paused
              workspaces remain until owner deletion; fixed disks cannot expand.
            </p>
          </section>
          {!saved ? (
            <FormField
              control={form.control}
              name="accepted"
              render={({ field }) => (
                <FormItem className="flex items-start gap-3">
                  <FormControl>
                    <Checkbox
                      checked={field.value}
                      onCheckedChange={(value) =>
                        field.onChange(value === true)
                      }
                      disabled={!acceptable || busy || quote.isPending}
                    />
                  </FormControl>
                  <div className="space-y-1">
                    <FormLabel>
                      I accept this exact rate, continuous account-balance
                      charging with no per-instance cap, and the displayed
                      profile, shared environment, and data policy versions.
                    </FormLabel>
                    <FormMessage />
                  </div>
                </FormItem>
              )}
            />
          ) : null}
          {unreadable ? (
            <ErrorBanner message="The saved request cannot be read. Review and reset it before continuing." />
          ) : null}
          {!permittedState && !saved ? (
            <ErrorBanner message="Paid resume is available only for a paused paid Machine. Reload its current details." />
          ) : null}
          {error ? <ErrorBanner message={error} /> : null}
          {conflict ? (
            <p role="alert" className="text-12 text-muted-foreground">
              The server rejected this request. Resolve the reported condition,
              review current Machine details and a fresh quote; continuing
              requires new acceptance. Adding funds does not start this request.
            </p>
          ) : null}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            {conflict || unreadable || (!saved && !quote.isPending) ? (
              <Button
                type="button"
                variant="outline"
                disabled={busy}
                onClick={reviewAgain}
              >
                Review fresh quote
              </Button>
            ) : null}
            <Button
              type="submit"
              isLoading={busy}
              disabled={!canSubmit || !form.watch("accepted")}
            >
              {saved ? "Retry saved paid request" : "Accept quote and resume"}
            </Button>
          </DialogFooter>
        </form>
      </Form>
    </DialogContent>
  );
}

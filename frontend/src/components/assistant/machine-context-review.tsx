import { useEffect, useState } from "react";
import { useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { SHARED_LEGACY_WARNING } from "@/schemas/machines";
import {
  type MachineAccess,
  type MachineContextReview as Review,
} from "@/schemas/machine-access";
import {
  usePrepareMachineContext,
  useMachineContextReview,
  useCommitMachineContext,
} from "@/hooks/use-machine-access";
import { MachineApplicationStatus } from "./machine-application-status";

const labels = {
  shell: "Shell commands",
  files: "File tools",
  browser: "Secure browser",
  computer: "Full computer control",
  developer_browser: "Developer browser",
};
const modeLabel = (mode: string) =>
  mode === "separated"
    ? "Separate workspace and browser"
    : mode === "shared_legacy"
      ? "Shared legacy"
      : "Owner selection required";
type Props = {
  readonly agentId: string;
  readonly machine: MachineAccess;
  readonly contextEnabled: boolean;
  readonly disabled: boolean;
  readonly unsavedCapabilities?: boolean;
  readonly configurationApplicability?: "hosted" | "not_applicable" | "unknown";
};

export function MachineContextReview(props: Props) {
  const owner = useAuthStore((state) => state.user?.id);
  return (
    <OwnerReview
      key={`${owner}:${props.agentId}:${props.machine.node_id}`}
      {...props}
    />
  );
}

function OwnerReview({
  agentId,
  machine,
  contextEnabled,
  disabled,
  unsavedCapabilities = false,
  configurationApplicability = "unknown",
}: Props) {
  const prepare = usePrepareMachineContext(agentId);
  const commit = useCommitMachineContext(agentId);
  const [prepared, setPrepared] = useState<Review>();
  const query = useMachineContextReview(
    agentId,
    machine.node_id,
    prepared?.review_id,
  );
  const review = query.data ?? prepared;
  const [error, setError] = useState<string>();
  const [conflict, setConflict] = useState(false);
  const [now, setNow] = useState(Date.now);
  const form = useAppForm({
    defaultValues: { sharedAccepted: false, acceptedReview: "" },
  });
  useEffect(() => {
    form.setValue("sharedAccepted", false, {
      shouldDirty: false,
      shouldTouch: false,
    });
    form.setValue("acceptedReview", "", {
      shouldDirty: false,
      shouldTouch: false,
    });
  }, [
    machine.revision,
    machine.machine_configuration?.revision,
    machine.machine_configuration?.digest,
    machine.machine_configuration?.runtime_generation,
    machine.machine_configuration?.state,
    machine.machine_configuration?.reason,
    machine.context_policy_version,
    machine.capability_edit_enabled,
    machine.context_setup_enabled,
    contextEnabled,
    machine.separated?.available,
    machine.separated?.reason,
    form,
  ]);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const support =
    machine.separated?.available === true && !machine.separated.reason;
  const contexts = contextEnabled && machine.context_setup_enabled;
  const configPending =
    machine.machine_configuration !== null &&
    (machine.machine_configuration.state !== "applied" ||
      machine.machine_configuration.reason !== null);
  const busy = prepare.isPending || commit.isPending;
  const modeDisabled =
    disabled ||
    !machine.can_edit ||
    !machine.protocol_v2 ||
    busy ||
    unsavedCapabilities ||
    configPending;
  const expired = review ? Date.parse(review.expires_at) <= now : false;
  const targetDrift = Boolean(
    review?.machine_configuration_target &&
    (!machine.machine_configuration ||
      review.machine_configuration_target.revision !==
        machine.machine_configuration.revision ||
      review.machine_configuration_target.digest !==
        machine.machine_configuration.digest ||
      review.machine_configuration_target.runtime_generation !==
        machine.machine_configuration.runtime_generation),
  );
  const stale =
    conflict ||
    (review &&
      review.status !== "committed" &&
      (!review.applicable ||
        expired ||
        review.status === "failed" ||
        review.expected_revision !== machine.revision ||
        targetDrift ||
        (review.mode === "separated" && (!contexts || !support))));
  // Confirmation belongs to exactly the server-authored scope displayed here.
  const fingerprint = review
    ? JSON.stringify({
        id: review.review_id,
        agent: review.agent_id,
        node: review.node_id,
        mode: review.mode,
        policy: review.policy_version,
        revision: review.expected_revision,
        capabilities: review.capabilities,
        logins: review.saved_login_ids,
        target: review.machine_configuration_target,
        added: review.capabilities_added,
        removed: review.capabilities_removed,
        loginsAdded: review.saved_login_ids_added,
        loginsRemoved: review.saved_login_ids_removed,
        warning: review.warning,
        storage: review.storage_note,
        expiry: review.expires_at,
      })
    : "";
  const accepted = Boolean(
    fingerprint && form.watch("acceptedReview") === fingerprint,
  );
  const canCommit = Boolean(
    review &&
    query.data &&
    !query.error &&
    !query.isFetching &&
    !stale &&
    ["pending", "approved"].includes(review.status) &&
    accepted &&
    !modeDisabled,
  );

  async function request(mode: Review["mode"]) {
    if (
      modeDisabled ||
      (mode === "separated"
        ? !contexts || !support
        : !form.getValues("sharedAccepted"))
    )
      return;
    setError(undefined);
    setConflict(false);
    form.setValue("acceptedReview", "", {
      shouldDirty: false,
      shouldTouch: false,
    });
    try {
      const result = await prepare.mutateAsync({
        node: machine.node_id,
        body: {
          mode,
          policy_version: machine.context_policy_version,
          acknowledge_separation: true,
        },
      });
      setPrepared(result);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not prepare the current machine mode review.",
      );
    }
  }
  async function approve() {
    if (!canCommit || !review) return;
    setError(undefined);
    try {
      const result = await commit.mutateAsync({
        node: machine.node_id,
        reviewId: review.review_id,
      });
      setPrepared(result);
      form.setValue("acceptedReview", "", {
        shouldDirty: false,
        shouldTouch: false,
      });
    } catch (cause) {
      const outdated =
        cause instanceof ApiError &&
        [400, 403, 404, 409, 410, 422].includes(cause.status);
      setConflict(outdated);
      form.setValue("acceptedReview", "", {
        shouldDirty: false,
        shouldTouch: false,
      });
      setError(
        outdated
          ? "Review is stale or no longer authorized; prepare and review again before applying."
          : "The commit result is unknown. Refresh this stored review before retrying; do not assume the mode was applied.",
      );
      void query.refetch();
    }
  }
  const tools = review
    ? (Object.keys(labels) as Array<keyof typeof labels>)
        .filter((key) => review.capabilities[key])
        .map((key) => labels[key])
    : [];
  return (
    <section
      aria-label="Owner environment mode controls"
      className="space-y-3 text-12"
    >
      <p>
        Change this Agent’s environment mode here. Prepare a stored owner
        review, inspect its exact tools and saved-login scope, then approve.
        This does not grant additional tools. Mode policy:{" "}
        {machine.context_policy_version}.
      </p>
      <p className="text-muted-foreground">
        Separate workspaces and browser profiles share the host kernel. Mode
        changes do not move shared files or copy cookies. Browser work waits for
        required profile cleanup.
      </p>
      {unsavedCapabilities ? (
        <p role="status">
          Save capability edits before requesting a mode change. Owner requests
          use the saved policy.
        </p>
      ) : null}
      {configPending ? (
        <p role="status" className="text-warning">
          Mode review waits for current hosted configuration evidence. Refresh
          machine status; Agent approval cannot apply the machine configuration.
        </p>
      ) : null}
      {machine.mode !== "separated" ? (
        <div className="space-y-2 rounded-lg border border-border p-3">
          {!contexts ? (
            <p role="status">
              Context setup is not enabled for this account. Existing separated
              assignments and restrictions still apply.
            </p>
          ) : null}
          {!support ? (
            <p role="status">
              Separated mode cannot be requested until current support is
              confirmed. Review the separation support reason above; no shared
              fallback is selected.
            </p>
          ) : null}
          <Button
            type="button"
            size="sm"
            variant="secondary"
            className="h-auto max-w-full whitespace-normal py-1 text-left"
            disabled={modeDisabled || !contexts || !support}
            onClick={() => void request("separated")}
          >
            Separate workspace and browser for this agent
          </Button>
        </div>
      ) : null}
      {machine.can_edit && machine.mode !== "shared_legacy" ? (
        <div className="space-y-2 rounded-lg border border-border p-3">
          <p>{SHARED_LEGACY_WARNING}</p>
          <label className="flex items-start gap-2">
            <Checkbox
              disabled={modeDisabled}
              checked={form.watch("sharedAccepted")}
              onCheckedChange={(value) =>
                form.setValue("sharedAccepted", value === true)
              }
            />
            I explicitly choose shared legacy access and acknowledge this
            warning.
          </label>
          <Button
            type="button"
            size="sm"
            variant="secondary"
            className="h-auto max-w-full whitespace-normal py-1 text-left"
            disabled={modeDisabled || !form.watch("sharedAccepted")}
            onClick={() => void request("shared_legacy")}
          >
            Review shared legacy mode
          </Button>
        </div>
      ) : null}
      {error ? (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      ) : null}
      {review ? (
        <section
          aria-label="Review environment mode change"
          className="space-y-3 rounded-lg border border-border p-3"
        >
          <div className="flex flex-wrap items-center gap-2">
            <h4 className="text-13 font-semibold">
              Review environment mode change
            </h4>
            <Badge
              variant={review.status === "failed" ? "destructive" : "secondary"}
            >
              {review.status}
            </Badge>
          </div>
          <p className="break-all">Stored review {review.review_id}</p>
          <p className="break-all">
            Agent: {review.agent_id} · Machine: {review.node_id}
          </p>
          <p>
            {modeLabel(review.previous_mode)} → {modeLabel(review.mode)}
          </p>
          <p>
            Reviewed policy: {review.policy_version} · Agent policy revision{" "}
            {review.expected_revision}
          </p>
          <p>
            {tools.length
              ? `Reviewed tools: ${tools.join(", ")}`
              : "Reviewed tools: None — all tools remain off"}
          </p>
          <p>
            Tools added: {review.capabilities_added.join(", ") || "None"};
            removed: {review.capabilities_removed.join(", ") || "None"}
          </p>
          <div>
            <p className="font-medium">Reviewed saved-login scope</p>
            {review.saved_login_ids === null ? (
              <p>
                No additional Machine login restriction; current Agent login
                policy still applies.
              </p>
            ) : review.saved_login_ids.length ? (
              <ul className="list-inside list-disc break-all">
                {review.saved_login_ids.map((id) => (
                  <li key={id}>{id}</li>
                ))}
              </ul>
            ) : (
              <p>None — no saved logins authorized</p>
            )}
          </div>
          <p>
            Saved logins added:{" "}
            {review.saved_login_ids_added.join(", ") || "None"}; removed:{" "}
            {review.saved_login_ids_removed.join(", ") || "None"}
          </p>
          {review.mode === "shared_legacy" ? <p>{review.warning}</p> : null}
          <p>{review.storage_note}</p>
          {review.machine_configuration_target ? (
            <div className="space-y-1">
              <p>
                Reviewed hosted configuration target: revision{" "}
                {review.machine_configuration_target.revision} · runtime
                generation{" "}
                {review.machine_configuration_target.runtime_generation} ·
                credential epoch{" "}
                {review.machine_configuration_target.credential_epoch}
              </p>
              <p className="break-all font-mono text-11">
                Reviewed configuration digest:{" "}
                {review.machine_configuration_target.digest}
              </p>
            </div>
          ) : null}
          <p>Review expires: {new Date(review.expires_at).toLocaleString()}</p>
          {query.isFetching ? (
            <p role="status">Refreshing stored owner review…</p>
          ) : null}
          {query.error ? (
            <p role="alert" className="text-destructive">
              The stored review could not be refreshed. Approval is unavailable
              until a successful refresh.
            </p>
          ) : null}
          {review.status === "approved" ? (
            <p role="status">
              Owner approval recorded. Authorization commit has not been
              confirmed.
            </p>
          ) : null}
          {stale ? (
            <p role="status" className="text-warning">
              This review is stale, expired or failed. Prepare and review the
              current scope again; the earlier confirmation cannot be reused.
            </p>
          ) : null}
          {review.applicability_reason || review.error_code ? (
            <details className="text-muted-foreground">
              <summary className="cursor-pointer">
                Review diagnostic information
              </summary>
              {review.applicability_reason ? (
                <p className="mt-1 break-words">
                  {review.applicability_reason.replaceAll("_", " ")}
                </p>
              ) : null}
              {review.error_code ? (
                <p className="mt-1 break-words">
                  {review.error_code.replaceAll("_", " ")}
                </p>
              ) : null}
            </details>
          ) : null}
          {review.status === "committed" ? (
            <>
              <p role="status">
                Authorization committed at revision {review.committed_revision}.
                This does not confirm runtime application.
              </p>
              <MachineApplicationStatus
                authorizationApplied={review.authorization_applied}
                environmentApplied={review.environment_applied}
                configuration={review.machine_configuration}
                target={review.machine_configuration_target}
                configurationApplicability={configurationApplicability}
              />
            </>
          ) : (
            <>
              <label className="flex items-start gap-2">
                <Checkbox
                  disabled={
                    modeDisabled ||
                    Boolean(stale) ||
                    Boolean(query.error) ||
                    query.isFetching
                  }
                  checked={accepted}
                  onCheckedChange={(value) =>
                    form.setValue(
                      "acceptedReview",
                      value === true ? fingerprint : "",
                    )
                  }
                />
                I approve this exact Agent, machine, mode, tool and saved-login
                scope under the displayed policy.
              </label>
              <Button
                type="button"
                variant="primary"
                className="h-auto whitespace-normal"
                disabled={!canCommit}
                isLoading={commit.isPending}
                onClick={() => void approve()}
              >
                Approve and apply reviewed mode
              </Button>
            </>
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              size="sm"
              variant="secondary"
              disabled={busy || query.isFetching}
              onClick={() => void query.refetch()}
            >
              Refresh stored review
            </Button>
            {stale ? (
              <Button
                type="button"
                size="sm"
                variant="secondary"
                disabled={modeDisabled}
                onClick={() => void request(review.mode)}
              >
                Prepare and review again
              </Button>
            ) : null}
          </div>
          <p className="text-muted-foreground">
            Status refreshes for up to one minute while this page is active. Use
            Refresh stored review for a later check.
          </p>
        </section>
      ) : null}
    </section>
  );
}

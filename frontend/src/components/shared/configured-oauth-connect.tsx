import { useId, useState } from "react";
import { Button } from "@/components/ui/button";
import { usePublicConfig } from "@/hooks/use-public-config";
import { useConfiguredConnect } from "@/hooks/use-configured-connect";

interface ConfiguredOAuthConnectProps {
  readonly userId: string;
  readonly serviceSlug: string;
  readonly serviceName: string;
  readonly returnPage: string;
  readonly pageOptions?: readonly { value: string; label: string }[];
  readonly showDestination?: boolean;
  readonly onContinue?: () => void;
  readonly continuing?: boolean;
}

export function ConfiguredOAuthConnect({
  userId,
  serviceSlug,
  serviceName,
  returnPage,
  pageOptions,
  showDestination = false,
  onContinue,
  continuing = false,
}: ConfiguredOAuthConnectProps) {
  const config = usePublicConfig();
  const enabled = config.data?.oauth_return_routes_enabled === true;
  const flow = useConfiguredConnect(userId, serviceSlug, returnPage);
  const [page, setPage] = useState(returnPage);
  const selectId = useId();
  const result = flow.verifiedStatus;

  if (!enabled && !flow.request && !flow.unmatchedReturn && !showDestination)
    return null;

  return (
    <section
      aria-label={`Connect ${serviceName}`}
      className="space-y-4 rounded-xl border border-border bg-card p-5 text-sm"
    >
      <div>
        <h3 className="font-medium">Connect {serviceName}</h3>
        <p className="mt-1 leading-6 text-muted-foreground">
          Authorize your account through NyxID, then return here to continue.
        </p>
      </div>
      {!enabled && !flow.request && (
        <p role="status">
          Configured return pages are not enabled on the connected backend.
        </p>
      )}
      {flow.unmatchedReturn && (
        <p role="alert" className="text-destructive">
          This return does not match a request saved for your account in this
          tab. Its result has not been accepted.
        </p>
      )}
      {!flow.request ? (
        <div className="flex flex-wrap items-end gap-3">
          {pageOptions && (
            <label className="grid gap-2" htmlFor={selectId}>
              Return page
              <select
                id={selectId}
                className="rounded-md border border-border bg-background p-2"
                value={page}
                onChange={(event) => setPage(event.target.value)}
              >
                {pageOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
          )}
          <Button
            type="button"
            disabled={!enabled || flow.preparing}
            onClick={() => flow.prepare(page)}
          >
            {showDestination ? "Prepare connection" : `Connect ${serviceName}`}
          </Button>
        </div>
      ) : (
        <div className="space-y-4">
          {showDestination && (
            <p className="break-all">
              Saved return destination:{" "}
              {flow.request.callback_url ?? "Not acknowledged by this backend"}
            </p>
          )}
          {!flow.request.callback_url && (
            <p role="alert" className="text-destructive">
              The backend did not confirm a configured destination. Cancel this
              request and retry after the backend rollout completes.
            </p>
          )}
          {flow.mismatchedStatus && (
            <p role="alert" className="text-destructive">
              The backend result does not match this {serviceName} connection
              request. It has not been accepted.
            </p>
          )}
          {flow.status.isPending && (
            <p role="status">Checking this connection request…</p>
          )}
          {result?.status === "pending" && (
            <p role="status">Waiting for authorization for this request.</p>
          )}
          {flow.completed && (
            <p role="status" className="text-emerald-500">
              {serviceName} connection confirmed by NyxID. You can continue.
            </p>
          )}
          {result?.status === "cancelled" && (
            <p role="status">Connection request cancelled.</p>
          )}
          {result?.status === "expired" && (
            <p role="status">
              Connection request expired. Start another request to retry.
            </p>
          )}
          {result?.status === "pending" && result.last_error && (
            <p role="alert" className="text-destructive">
              Authorization was not completed. You can reopen setup to retry or
              cancel this request.{showDestination && ` (${result.last_error})`}
            </p>
          )}
          <div className="flex flex-wrap items-center gap-3">
            {flow.canOpen && (
              <a
                className="font-medium underline underline-offset-4"
                href={flow.request.connect_url}
              >
                Open {serviceName} setup
              </a>
            )}
            {!flow.terminal && (
              <Button
                type="button"
                variant="outline"
                disabled={flow.cancelling}
                onClick={flow.cancelRequest}
              >
                Cancel request
              </Button>
            )}
            {flow.completed && result?.connected_service && (
              <a
                className="font-medium underline underline-offset-4"
                href={`/keys/${encodeURIComponent(result.connected_service.id)}`}
              >
                View connected service
              </a>
            )}
            {flow.completed && onContinue && (
              <Button type="button" disabled={continuing} onClick={onContinue}>
                Continue onboarding
              </Button>
            )}
            {flow.terminal && (
              <Button type="button" variant="outline" onClick={flow.finish}>
                Start another request
              </Button>
            )}
            <Button
              type="button"
              variant="outline"
              disabled={flow.status.isFetching}
              onClick={() => void flow.status.refetch()}
            >
              Refresh result
            </Button>
          </div>
        </div>
      )}
      {flow.error && (
        <p role="alert" className="text-destructive">
          {flow.error}
        </p>
      )}
    </section>
  );
}

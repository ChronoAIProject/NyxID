import { useState } from "react";
import {
  useChannelActivities,
  useActivityCallback,
  useSetActivityCallback,
} from "@/hooks/use-channel-activities";
import type {
  ChannelActivityDescriptor,
  ChannelActivityItem,
} from "@/types/channels";
import { formatRelativeTime } from "@/lib/utils";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

function activityLabel(
  kind: string,
  descriptors: readonly ChannelActivityDescriptor[],
) {
  return (
    descriptors.find((entry) => entry.kind === kind)?.label ??
    (kind === "message" ? "Message" : kind)
  );
}

export function LatestActivity({
  activity,
  descriptors,
  unavailable,
  loading,
}: {
  readonly activity?: ChannelActivityItem;
  readonly descriptors: readonly ChannelActivityDescriptor[];
  readonly unavailable?: boolean;
  readonly loading?: boolean;
}) {
  if (unavailable) return <span>Activity unavailable</span>;
  if (loading) return <span>Loading activity…</span>;
  if (!activity) return <span>No activity received</span>;
  return (
    <span>
      {activityLabel(activity.kind, descriptors)} ·{" "}
      {formatRelativeTime(activity.received_at)}
    </span>
  );
}

function callbackLabel(status: string | null) {
  switch (status) {
    case "delivered":
      return "Callback accepted";
    case "failed":
      return "Callback failed";
    case "timeout":
      return "Callback timed out";
    case "not_enabled":
      return "Agent notification not enabled";
    case "pending":
      return "Callback pending";
    default:
      return "Callback status unavailable";
  }
}

export function ChannelActivities({
  scope,
  id,
  descriptors,
}: {
  readonly scope: "bot" | "route";
  readonly id: string;
  readonly descriptors: readonly ChannelActivityDescriptor[];
}) {
  const [kind, setKind] = useState("all");
  const [page, setPage] = useState(1);
  const query = useChannelActivities(
    scope,
    id,
    true,
    kind === "all" ? "" : kind,
    page,
  );
  return (
    <DetailSection title="Received activities">
      <div className="space-y-4 p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-xs text-muted-foreground">
            {query.error
              ? "Activity count unavailable"
              : query.isPending
                ? "Loading activity count…"
                : `${query.data.total} ${kind === "all" ? "received" : "matching"} ${query.data.total === 1 ? "activity" : "activities"} in the last ${query.data.retention_days} days`}
          </p>
          <Select
            value={kind}
            onValueChange={(value) => {
              setKind(value);
              setPage(1);
            }}
          >
            <SelectTrigger aria-label="Activity type" className="w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All activity types</SelectItem>
              {descriptors.map((entry) => (
                <SelectItem key={entry.kind} value={entry.kind}>
                  {entry.label}
                </SelectItem>
              ))}
              <SelectItem value="message">Earlier messages</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <p className="text-xs text-muted-foreground">
          Received means NyxID recorded the activity. Callback acceptance means
          the agent acknowledged it; it does not confirm processing. Message
          content is not stored here.
        </p>
        {query.error ? (
          <ErrorBanner
            message="Activities could not be loaded."
            onRetry={() => void query.refetch()}
          />
        ) : query.isPending ? (
          <p className="text-xs text-muted-foreground">Loading activities…</p>
        ) : !query.data.activities.length ? (
          <p className="text-xs text-muted-foreground">
            No matching activity received in this period.
          </p>
        ) : (
          <ul className="divide-y divide-border">
            {query.data.activities.map((activity) => (
              <li key={activity.id} className="space-y-2 py-3">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant="secondary">
                    {activityLabel(activity.kind, descriptors)}
                  </Badge>
                  <span className="text-xs text-muted-foreground">
                    {formatRelativeTime(activity.received_at)}
                  </span>
                  <span className="text-xs">
                    {callbackLabel(activity.callback_status)}
                  </span>
                </div>
                <p className="break-all text-xs text-muted-foreground">
                  From {activity.sender_platform_id ?? "unknown sender"} ·{" "}
                  {activity.platform_conversation_id ?? "unknown conversation"}
                </p>
                {activity.content_availability === "encrypted" && (
                  <p className="text-xs text-muted-foreground">
                    Encrypted message received. Text and replies are
                    unavailable.
                  </p>
                )}
                {activity.content_availability === "metadata_only" && (
                  <p className="text-xs text-muted-foreground">
                    Metadata notification. Replies are unavailable.
                  </p>
                )}
              </li>
            ))}
          </ul>
        )}
        {(page > 1 ||
          (query.data && query.data.total > query.data.per_page)) && (
          <div className="flex justify-end gap-2">
            <Button
              variant="outline"
              disabled={page === 1 || query.isFetching}
              onClick={() => setPage(page - 1)}
            >
              Previous
            </Button>
            <Button
              variant="outline"
              disabled={
                !query.data ||
                page * query.data.per_page >= query.data.total ||
                query.isFetching ||
                !!query.error
              }
              onClick={() => setPage(page + 1)}
            >
              Next
            </Button>
          </div>
        )}
      </div>
    </DetailSection>
  );
}

export function ActivityCallbackSettings({
  id,
  descriptors,
}: {
  readonly id: string;
  readonly descriptors: readonly ChannelActivityDescriptor[];
}) {
  const query = useActivityCallback(id);
  const update = useSetActivityCallback(id);
  return (
    <DetailSection title="Typed activity callbacks">
      <div className="space-y-3 p-4">
        <p className="text-xs text-muted-foreground">
          Enable activity types only after your agent declares support for this
          route. Encrypted chat and own-post notifications contain metadata and
          cannot be replied to. Enabling applies to new activity; earlier
          notifications are not replayed.
        </p>
        {query.error ? (
          <ErrorBanner message="Agent activity support could not be loaded." />
        ) : query.isPending ? (
          <p className="text-xs text-muted-foreground">
            Loading agent support…
          </p>
        ) : (
          <>
            <p className="text-xs">
              {query.data.declared
                ? `Agent supports: ${query.data.kinds.map((kind) => activityLabel(kind, descriptors)).join(", ")}`
                : "The assigned agent has not declared support for its current callback. Existing callbacks continue normally."}
            </p>
            <Button
              disabled={
                update.isPending ||
                (!query.data.enabled && !query.data.declared)
              }
              isLoading={update.isPending}
              onClick={() => update.mutate(!query.data.enabled)}
            >
              {query.data.enabled
                ? "Disable typed callbacks"
                : "Enable typed callbacks"}
            </Button>
          </>
        )}
        {update.error && <ErrorBanner message={update.error.message} />}
      </div>
    </DetailSection>
  );
}

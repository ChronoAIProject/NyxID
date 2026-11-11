import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
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
  useUiTranslation();
  if (unavailable) return <span>{translateUi("Activity unavailable")}</span>;
  if (loading) return <span>{translateUi("Loading activity…")}</span>;
  if (!activity) return <span>{translateUi("No activity received")}</span>;
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
      return translateUi("Callback accepted");
    case "failed":
      return translateUi("Callback failed");
    case "timeout":
      return translateUi("Callback timed out");
    case "not_enabled":
      return translateUi("Agent notification not enabled");
    case "pending":
      return translateUi("Callback pending");
    default:
      return translateUi("Callback status unavailable");
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
  useUiTranslation();
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
    <DetailSection title={translateUi("Received activities")}>
      <div className="space-y-4 p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-13 text-muted-foreground">
            {query.error
              ? translateUi("Activity count unavailable")
              : query.isPending
                ? translateUi("Loading activity count…")
                : kind === "all" ? translateUi("{{count}} received activities in the last {{days}} days", { count: query.data.total, days: query.data.retention_days }) : translateUi("{{count}} matching activities in the last {{days}} days", { count: query.data.total, days: query.data.retention_days })}
          </p>
          <Select
            value={kind}
            onValueChange={(value) => {
              setKind(value);
              setPage(1);
            }}
          >
            <SelectTrigger aria-label={translateUi("Activity type")} className="w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{translateUi("All activity types")}</SelectItem>
              {descriptors.map((entry) => (
                <SelectItem key={entry.kind} value={entry.kind}>
                  {entry.label}
                </SelectItem>
              ))}
              <SelectItem value="message">{translateUi("Earlier messages")}</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <p className="text-13 text-muted-foreground">{translateUi("Received means NyxID recorded the activity. Callback acceptance means the agent acknowledged it; it does not confirm processing. Message content is not stored here.")}</p>
        {query.error ? (
          <ErrorBanner
            message={translateUi("Activities could not be loaded.")}
            onRetry={() => void query.refetch()}
          />
        ) : query.isPending ? (
          <p className="text-13 text-muted-foreground">{translateUi("Loading activities…")}</p>
        ) : !query.data.activities.length ? (
          <p className="text-13 text-muted-foreground">{translateUi("No matching activity received in this period.")}</p>
        ) : (
          <ul className="divide-y divide-border">
            {query.data.activities.map((activity) => (
              <li key={activity.id} className="space-y-2 py-3">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant="secondary">
                    {activityLabel(activity.kind, descriptors)}
                  </Badge>
                  <span className="text-13 text-muted-foreground">
                    {formatRelativeTime(activity.received_at)}
                  </span>
                  <span className="text-13">
                    {callbackLabel(activity.callback_status)}
                  </span>
                </div>
                <p className="break-all text-13 text-muted-foreground">{translateUi("From ")}{activity.sender_platform_id ?? translateUi("unknown sender")} ·{" "}
                  {activity.platform_conversation_id ?? translateUi("unknown conversation")}
                </p>
                {activity.nyxbot_admission && (
                  <div className="space-y-1 break-all text-13 text-muted-foreground" aria-label={translateUi("NyxBot admission")}>
                    <p>{translateUi("NyxBot admission: ")}{activity.nyxbot_admission.disposition.replaceAll("_", " ")}</p>
                    <p>{translateUi("Bot mentioned: ")}{activity.nyxbot_admission.bot_mentioned === null ? translateUi("unknown") : activity.nyxbot_admission.bot_mentioned ? translateUi("yes") : translateUi("no")}
                      {activity.nyxbot_admission.managed_role_mentioned ? translateUi(" (own managed role)") : ""}{translateUi(" · Broadcast roles: ")}{activity.nyxbot_admission.broadcast_roles ?? translateUi("unknown")}
                      {activity.nyxbot_admission.broadcast_everyone_or_here ? translateUi(" · @everyone/@here") : ""}</p>
                    <p>{translateUi("Address: ")}{activity.nyxbot_admission.address}{translateUi(" · Thread follow: ")}{activity.nyxbot_admission.thread_follow_selection ?? translateUi("not evaluated")}</p>
                    {activity.nyxbot_admission.resolved_thread_id && <p>{translateUi("Thread: ")}{activity.nyxbot_admission.resolved_thread_id}</p>}
                    <p>{activity.nyxbot_admission.turn_id ? translateUi("Turn: {{value1}}", { value1: activity.nyxbot_admission.turn_id }) : translateUi("No agent turn started.")}</p>
                  </div>
                )}
                {activity.content_availability === "encrypted" && (
                  <p className="text-13 text-muted-foreground">{translateUi("Encrypted message received. Text and replies are unavailable.")}</p>
                )}
                {activity.content_availability === "metadata_only" && (
                  <p className="text-13 text-muted-foreground">{translateUi("Metadata notification. Replies are unavailable.")}</p>
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
            >{translateUi("Previous")}</Button>
            <Button
              variant="outline"
              disabled={
                !query.data ||
                page * query.data.per_page >= query.data.total ||
                query.isFetching ||
                !!query.error
              }
              onClick={() => setPage(page + 1)}
            >{translateUi("Next")}</Button>
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
  useUiTranslation();
  const query = useActivityCallback(id);
  const update = useSetActivityCallback(id);
  return (
    <DetailSection title={translateUi("Typed activity callbacks")}>
      <div className="space-y-3 p-4">
        <p className="text-13 text-muted-foreground">{translateUi("Enable activity types only after your agent declares support for this route. Encrypted chat and own-post notifications contain metadata and cannot be replied to. Enabling applies to new activity; earlier notifications are not replayed.")}</p>
        {query.error ? (
          <ErrorBanner message={translateUi("Agent activity support could not be loaded.")} />
        ) : query.isPending ? (
          <p className="text-13 text-muted-foreground">{translateUi("Loading agent support…")}</p>
        ) : (
          <>
            <p className="text-13">
              {query.data.declared
                ? translateUi("Agent supports: {{value1}}", { value1: query.data.kinds.map((kind) => activityLabel(kind, descriptors)).join(", ") })
                : translateUi("The assigned agent has not declared support for its current callback. Existing callbacks continue normally.")}
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
                ? translateUi("Disable typed callbacks")
                : translateUi("Enable typed callbacks")}
            </Button>
          </>
        )}
        {update.error && <ErrorBanner message={update.error.message} />}
      </div>
    </DetailSection>
  );
}

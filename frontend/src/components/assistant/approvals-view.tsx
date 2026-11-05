import { useEffect, useState } from "react";
import { RefreshCw, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { ApprovalCard } from "@/components/assistant/blocks/approval-card";
import { ApprovalAction, ApprovalIdentity } from "@/components/shared/approval-history-fields";
import { ServiceIcon } from "@/components/service-icon";
import { Skeleton } from "@/components/ui/skeleton";
import { GroupedList, GroupedRow } from "@/components/ui/grouped-list";
import {
  SegmentedControl,
  SegmentedControlList,
  SegmentedControlTrigger,
  SegmentedControlContent,
} from "@/components/ui/segmented-control";
import { WorkspacePageHeader } from "./workspace-page-header";
import { useDecideApproval } from "@/hooks/use-approvals";
import { useAssistantApprovals } from "@/hooks/use-assistant-workspace";
import { ApiError } from "@/lib/api-client";
import type { AssistantApprovalEntry } from "@/lib/assistant/approvals";
import type { ApprovalDecisionChannel } from "@/types/assistant";

function channelLabel(channel: ApprovalDecisionChannel | null): string {
  return channel === "web"
    ? "Web"
    : channel === "telegram"
      ? "Telegram"
      : channel === "mobile"
        ? "Mobile"
        : "";
}

function timestamp(value: string | null | undefined): number | null {
  const parsed = value ? Date.parse(value) : NaN;
  return Number.isFinite(parsed) ? parsed : null;
}

/** Missing and malformed timestamps always follow dated entries. */
function compareIsoDates(
  left: string | null | undefined,
  right: string | null | undefined,
  direction: "ascending" | "descending",
) {
  const a = timestamp(left),
    b = timestamp(right);
  if (a === null) return b === null ? 0 : 1;
  if (b === null) return -1;
  return direction === "ascending" ? a - b : b - a;
}

function dayLabel(value: string | null | undefined) {
  const time = timestamp(value);
  if (time === null) return "Unknown date";
  const date = new Date(time);
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (date.toDateString() === today.toDateString()) return "Today";
  if (date.toDateString() === yesterday.toDateString()) return "Yesterday";
  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function EmptyState({ history = false }: { readonly history?: boolean }) {
  return (
    <div className="flex flex-col items-center py-12 text-center">
      <ShieldCheck aria-hidden className="mb-3 size-6 text-text-tertiary" />
      <p className="type-card-title">
        {history ? "No decisions yet" : "Nothing waiting on you"}
      </p>
      <p className="mt-1 text-13 text-text-tertiary">
        {history
          ? "Decided approvals will show up here."
          : "Approval requests appear when an action needs your sign-off."}
      </p>
    </div>
  );
}

function HistorySection({
  entries,
}: {
  readonly entries: readonly AssistantApprovalEntry[];
}) {
  if (!entries.length) return <EmptyState history />;
  const days = new Map<string, AssistantApprovalEntry[]>();
  for (const entry of entries) {
    const day = dayLabel(entry.decidedAt ?? entry.requestedAt);
    days.set(day, [...(days.get(day) ?? []), entry]);
  }
  return (
    <div className="space-y-6">
      {[...days].map(([day, rows]) => (
        <GroupedList key={day} title={day}>
          {rows.map((entry) => {
            const decision = entry.block.decision;
            const at = timestamp(entry.decidedAt ?? entry.requestedAt);
            return (
              <GroupedRow
                key={entry.requestId}
                leading={
                  <ServiceIcon
                    slug={`api-${entry.block.service_slug}`}
                    size="sm"
                  />
                }
                title={<>
                  <ApprovalIdentity className="block">{entry.serviceName}</ApprovalIdentity>
                  <ApprovalAction
                    className="mt-1 line-clamp-2 break-words"
                    title={entry.block.body}
                  >
                    {entry.block.body}
                  </ApprovalAction>
                  <p className="mt-1 type-secondary text-text-tertiary">
                    <span
                      className={
                        decision === "approved"
                          ? "workspace-status-success"
                          : decision === "denied"
                            ? "workspace-status-destructive"
                            : undefined
                      }
                    >
                      {decision
                        ? decision.charAt(0).toUpperCase() + decision.slice(1)
                        : "Pending"}
                    </span>
                    {[
                      channelLabel(entry.block.decision_channel),
                      at === null
                        ? ""
                        : new Date(at).toLocaleTimeString(undefined, {
                            hour: "numeric",
                            minute: "2-digit",
                          }),
                      entry.block.agent_key_prefix,
                    ]
                      .filter(Boolean)
                      .map((part, i) => (
                        <span key={i}> · {part}</span>
                      ))}
                  </p>
                </>}
              />
            );
          })}
        </GroupedList>
      ))}
    </div>
  );
}

export function ApprovalsView() {
  const approvals = useAssistantApprovals();
  const [tab, setTab] = useState("pending");
  useEffect(() => {
    if (!approvals.isError) return;
    toast.error("Could not load approvals", {
      id: "assistant-approvals-load-failed",
      description:
        "The assistant backend did not respond. Deciding from Telegram or mobile still works — retry below.",
    });
  }, [approvals.isError]);
  const decide = useDecideApproval();
  const pending = [...approvals.pending].sort((a, b) =>
    compareIsoDates(a.block.expires_at, b.block.expires_at, "ascending"),
  );
  const decided = [...approvals.history].sort((a, b) =>
    compareIsoDates(
      a.decidedAt ?? a.requestedAt,
      b.decidedAt ?? b.requestedAt,
      "descending",
    ),
  );

  async function handleDecide(
    entry: AssistantApprovalEntry,
    approved: boolean,
  ): Promise<void> {
    try {
      await decide.mutateAsync({ requestId: entry.requestId, approved });
      toast.success(approved ? "Request approved" : "Request denied");
    } catch (error) {
      toast.error(
        error instanceof ApiError
          ? error.message
          : `Failed to ${approved ? "approve" : "deny"} the request`,
      );
    }
  }

  return (
    <div className="shell-page-stack">
      <WorkspacePageHeader
        column
        title="Approvals"
        description="Review actions waiting for your permission. Decisions here, in Telegram or on mobile stay in sync."
      />
      <SegmentedControl
        value={tab}
        onValueChange={setTab}
        className="max-w-2xl space-y-6"
      >
        <SegmentedControlList aria-label="Approval status" className="max-w-80">
          <SegmentedControlTrigger value="pending">
            Pending ({pending.length})
          </SegmentedControlTrigger>
          <SegmentedControlTrigger value="history">
            History
          </SegmentedControlTrigger>
        </SegmentedControlList>
        {approvals.isError ? (
          <GroupedList footer="Approvals may be out of date until the backend responds.">
            <GroupedRow asChild interactive>
              <button onClick={approvals.refetch}>
                <RefreshCw aria-hidden />
                Retry
              </button>
            </GroupedRow>
          </GroupedList>
        ) : null}
        {approvals.isLoading ? (
          <div
            role="status"
            aria-label="Loading approvals"
            className="space-y-3"
          >
            {Array.from({ length: 3 }, (_, i) => (
              <Skeleton key={i} className="h-24 w-full" />
            ))}
          </div>
        ) : (
          <>
            <SegmentedControlContent
              value="pending"
              className="space-y-6"
            >
              {pending.length ? (
                pending.map((entry) => (
                  <article
                    key={entry.requestId}
                    aria-label={`${entry.serviceName} request`}
                    className="assistant-workspace-approval space-y-2"
                  >
                    <h3 className="flex items-center gap-2 px-4 type-row-title">
                      <ServiceIcon
                        slug={`api-${entry.block.service_slug}`}
                        size="xs"
                      />
                      {entry.serviceName}
                    </h3>
                    <ApprovalCard
                      block={entry.block}
                      onDecide={(approved) => handleDecide(entry, approved)}
                    />
                  </article>
                ))
              ) : (
                <EmptyState />
              )}
            </SegmentedControlContent>
            <SegmentedControlContent value="history">
              <HistorySection entries={decided} />
            </SegmentedControlContent>
          </>
        )}
      </SegmentedControl>
    </div>
  );
}

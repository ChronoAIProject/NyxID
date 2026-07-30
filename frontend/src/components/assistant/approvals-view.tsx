import { ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { ApprovalCard } from "@/components/assistant/blocks/approval-card";
import { ServiceIcon } from "@/components/service-icon";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useDecideApproval } from "@/hooks/use-approvals";
import { useAssistantApprovals } from "@/hooks/use-assistant";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import type { AssistantApprovalEntry } from "@/lib/assistant/approvals";
import type {
  ApprovalDecision,
  ApprovalDecisionChannel,
} from "@/types/assistant";

// Decision vocabulary mirrors the Studio approval-history page (badge variants
// per status); "denied" is the assistant-block spelling of NyxID "rejected"
// (PRD §5.5).
function getDecisionBadge(decision: ApprovalDecision) {
  switch (decision) {
    case "approved":
      return <Badge variant="success">Approved</Badge>;
    case "denied":
      return <Badge variant="destructive">Denied</Badge>;
    case "expired":
      return <Badge variant="secondary">Expired</Badge>;
    case "cancelled":
      return <Badge variant="secondary">Cancelled</Badge>;
  }
}

function channelLabel(channel: ApprovalDecisionChannel | null): string {
  switch (channel) {
    case "web":
      return "Web";
    case "telegram":
      return "Telegram";
    case "mobile":
      return "Mobile";
    default:
      return "—";
  }
}

/** ISO-8601 comparator that always sorts absent values after present ones. */
function compareIsoDates(
  left: string | null | undefined,
  right: string | null | undefined,
  direction: "ascending" | "descending",
): number {
  if (!left) return right ? 1 : 0;
  if (!right) return -1;
  return direction === "ascending"
    ? left.localeCompare(right)
    : right.localeCompare(left);
}

function ServiceCell({ slug }: { readonly slug: string }) {
  return (
    <span className="inline-flex items-center gap-1.5 whitespace-nowrap">
      <ServiceIcon slug={`api-${slug}`} size="xs" />
      {slug}
    </span>
  );
}

function PendingSection({
  entries,
  onDecide,
}: {
  readonly entries: readonly AssistantApprovalEntry[];
  readonly onDecide: (
    entry: AssistantApprovalEntry,
    approved: boolean,
  ) => Promise<void>;
}) {
  if (entries.length === 0) {
    return (
      <div className="flex flex-col items-center rounded-xl border border-border bg-card px-6 py-12 text-center">
        <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-success/25 bg-success/[0.06]">
          <ShieldCheck className="h-5 w-5 text-success" />
        </div>
        <p className="mt-3 text-[13px] font-medium text-foreground">
          Nothing waiting on you
        </p>
        <p className="mt-1 text-[11px] text-muted-foreground">
          Approval requests show up here when a write needs your sign-off.
        </p>
      </div>
    );
  }

  return (
    <>
      {/* A div, not a p: `Badge` renders a div, which is invalid inside a p and
          gets reparented by the browser. */}
      <div className="mb-2.5 flex items-center gap-2 text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
        Waiting on you
        <Badge variant="warning">{entries.length}</Badge>
      </div>
      <div className="space-y-5">
        {entries.map((entry) => (
          <div key={entry.requestId}>
            <p className="mb-2 flex items-center gap-1.5 text-[12px] font-medium text-muted-foreground">
              <ServiceIcon slug={`api-${entry.block.service_slug}`} size="xs" />
              <span className="truncate">{entry.serviceName}</span>
            </p>
            <ApprovalCard
              block={entry.block}
              onDecide={(approved) => onDecide(entry, approved)}
            />
          </div>
        ))}
      </div>
    </>
  );
}

function HistorySection({
  entries,
}: {
  readonly entries: readonly AssistantApprovalEntry[];
}) {
  if (entries.length === 0) {
    return (
      <div className="rounded-lg bg-overlay px-4 py-3 text-[12px] text-muted-foreground">
        Decided approvals will show up here.
      </div>
    );
  }

  return (
    <>
      {/* Mobile card view */}
      <div className="flex flex-col gap-3 md:hidden">
        {entries.map((entry) => (
          <div
            key={entry.requestId}
            className="rounded-xl border border-border/50 bg-card p-4"
          >
            <div className="flex items-start justify-between gap-2">
              <p className="min-w-0 text-[13px] font-medium text-foreground line-clamp-2">
                {entry.block.body}
              </p>
              {entry.block.decision !== null &&
                getDecisionBadge(entry.block.decision)}
            </div>
            <p className="mt-1.5 flex items-center gap-1.5 text-[11px] text-muted-foreground">
              <ServiceCell slug={entry.block.service_slug} />
              {entry.block.decision_channel !== null &&
                `via ${channelLabel(entry.block.decision_channel)}`}
            </p>
            <div className="mt-3 flex flex-wrap items-center justify-between gap-x-4 gap-y-1">
              <span className="text-[11px] text-muted-foreground">
                {formatDate(entry.decidedAt ?? entry.requestedAt)}
              </span>
              <span className="text-[11px] text-text-tertiary">
                {entry.block.agent_key_prefix}
              </span>
            </div>
          </div>
        ))}
      </div>

      {/* Desktop table view */}
      <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Request</TableHead>
              <TableHead>Service</TableHead>
              <TableHead>Decision</TableHead>
              <TableHead>Channel</TableHead>
              <TableHead>When</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {entries.map((entry) => (
              <TableRow key={entry.requestId}>
                <TableCell>
                  <div className="flex min-w-0 flex-col gap-0.5">
                    <span
                      className="max-w-[260px] truncate"
                      title={entry.block.body}
                    >
                      {entry.block.body}
                    </span>
                    <span className="text-[11px] text-text-tertiary">
                      {entry.block.agent_key_prefix}
                    </span>
                  </div>
                </TableCell>
                <TableCell className="text-muted-foreground">
                  <ServiceCell slug={entry.block.service_slug} />
                </TableCell>
                <TableCell>
                  {entry.block.decision !== null &&
                    getDecisionBadge(entry.block.decision)}
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {channelLabel(entry.block.decision_channel)}
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {formatDate(entry.decidedAt ?? entry.requestedAt)}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </>
  );
}

export function ApprovalsView() {
  const approvals = useAssistantApprovals();
  const decide = useDecideApproval();

  // Pending: most urgent (soonest expiry) first. History: newest first.
  // Both keys are ISO strings the API always sends, but a comparator that
  // throws takes the whole route down with it (the router unmounts to a blank
  // page), so one absent timestamp must degrade to "sorts last" instead.
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
    <div className="h-full min-h-0 overflow-y-auto overscroll-contain">
      <div className="px-5 pt-6 sm:px-8">
        <h1 className="text-[22px] font-bold tracking-[-0.03em] sm:text-[28px]">
          Approvals
        </h1>
        <p className="mt-1 max-w-2xl text-[12px] text-muted-foreground">
          Write actions gated by your NyxID policy wait here for your decision.
          Deciding here, in Telegram, or on mobile converges to the same result.
        </p>
      </div>

      <div className="max-w-[680px] px-5 pb-10 pt-5 sm:px-8">
        {approvals.isLoading ? (
          <div className="space-y-3">
            {Array.from({ length: 3 }).map((_, i) => (
              <Skeleton
                key={`approval-skel-${String(i)}`}
                className="h-24 w-full"
              />
            ))}
          </div>
        ) : approvals.isError ? (
          <ErrorBanner
            message="Failed to load approvals. Please try again."
            onRetry={approvals.refetch}
          />
        ) : (
          <>
            <PendingSection entries={pending} onDecide={handleDecide} />

            <p className="mb-2.5 mt-8 text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
              History
            </p>
            <HistorySection entries={decided} />
          </>
        )}
      </div>
    </div>
  );
}

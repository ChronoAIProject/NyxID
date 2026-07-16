import { Link } from "@tanstack/react-router";
import { ArrowUpRight, ShieldCheck } from "lucide-react";
import { ApprovalCard } from "@/components/assistant/blocks/approval-card";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useApprovals, useDecideApprovalFor } from "@/hooks/use-assistant";
import { formatDate } from "@/lib/utils";
import type { ApprovalListEntry } from "@/lib/assistant/mock-data";
import type {
  ApprovalCardContentBlock,
  ApprovalDecision,
  ApprovalDecisionChannel,
} from "@/types/assistant";

// Decision vocabulary mirrors the Studio approval-history page (badge variants
// per status); "denied"/"cancelled" are the assistant-block spellings of the
// same outcomes (PRD §5.5: NyxID "rejected" ⇄ block "denied").
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
      return "-";
  }
}

function ConversationLink({
  conversationId,
  conversationTitle,
  className,
}: {
  readonly conversationId: string;
  readonly conversationTitle: string;
  readonly className?: string;
}) {
  return (
    <Link
      to="/assistant"
      search={{ c: conversationId }}
      className={`group inline-flex items-center gap-1 text-[11px] text-text-tertiary transition-colors hover:text-foreground ${className ?? ""}`}
    >
      {conversationTitle}
      <ArrowUpRight className="h-3 w-3 opacity-0 transition-opacity group-hover:opacity-100" />
    </Link>
  );
}

function ApprovalEntry({
  conversationId,
  conversationTitle,
  block,
  onDecide,
}: {
  readonly conversationId: string;
  readonly conversationTitle: string;
  readonly block: ApprovalCardContentBlock;
  readonly onDecide: (approved: boolean) => Promise<void>;
}) {
  return (
    <div>
      <ConversationLink
        conversationId={conversationId}
        conversationTitle={conversationTitle}
        className="mb-1.5"
      />
      <ApprovalCard block={block} onDecide={onDecide} />
    </div>
  );
}

function HistorySection({ entries }: { readonly entries: ApprovalListEntry[] }) {
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
            key={entry.block.block_id}
            className="rounded-xl border border-border/50 bg-card p-4"
          >
            <div className="flex items-start justify-between gap-2">
              <p className="min-w-0 text-[13px] font-medium text-foreground line-clamp-2">
                {entry.block.body}
              </p>
              {entry.block.decision !== null &&
                getDecisionBadge(entry.block.decision)}
            </div>
            <p className="mt-1 text-[11px] text-muted-foreground">
              {entry.block.service_slug}
              {entry.block.decision_channel !== null &&
                ` - via ${channelLabel(entry.block.decision_channel)}`}
            </p>
            <div className="mt-3 flex flex-wrap items-center justify-between gap-x-4 gap-y-1">
              <span className="text-[11px] text-muted-foreground">
                {formatDate(entry.requestedAt)}
              </span>
              <ConversationLink
                conversationId={entry.conversationId}
                conversationTitle={entry.conversationTitle}
              />
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
              <TableRow key={entry.block.block_id}>
                <TableCell>
                  <div className="flex min-w-0 flex-col gap-0.5">
                    <span className="max-w-[260px] truncate">
                      {entry.block.body}
                    </span>
                    <ConversationLink
                      conversationId={entry.conversationId}
                      conversationTitle={entry.conversationTitle}
                    />
                  </div>
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {entry.block.service_slug}
                </TableCell>
                <TableCell>
                  {entry.block.decision !== null &&
                    getDecisionBadge(entry.block.decision)}
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {channelLabel(entry.block.decision_channel)}
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {formatDate(entry.requestedAt)}
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
  const approvals = useApprovals();
  const decide = useDecideApprovalFor();
  const entries = approvals.data ?? [];
  const pending = entries.filter((entry) => entry.block.decision === null);
  const decided = entries.filter((entry) => entry.block.decision !== null);

  return (
    <div className="h-full min-h-0 overflow-y-auto overscroll-contain">
      <div className="px-5 pt-6 sm:px-8">
        <h1 className="text-[22px] font-bold tracking-[-0.03em] sm:text-[28px]">
          Approvals
        </h1>
        <p className="mt-1 max-w-2xl text-[12px] text-muted-foreground">
          Write actions gated by your NyxID policy wait here for your decision.
          Deciding in chat, Telegram, or mobile converges to the same result.
        </p>
      </div>

      <div className="max-w-[680px] px-5 pb-10 pt-5 sm:px-8">
        {pending.length > 0 ? (
          <>
            <p className="mb-2.5 text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
              Waiting on you
            </p>
            <div className="space-y-4">
              {pending.map((entry) => (
                <ApprovalEntry
                  key={entry.block.block_id}
                  conversationId={entry.conversationId}
                  conversationTitle={entry.conversationTitle}
                  block={entry.block}
                  onDecide={(approved) =>
                    decide.mutateAsync({
                      conversationId: entry.conversationId,
                      blockId: entry.block.block_id,
                      approved,
                    })
                  }
                />
              ))}
            </div>
          </>
        ) : (
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
        )}

        <p className="mb-2.5 mt-8 text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
          History
        </p>
        <HistorySection entries={decided} />
      </div>
    </div>
  );
}

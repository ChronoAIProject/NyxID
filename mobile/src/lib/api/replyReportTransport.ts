import type { RequestOptions } from "./authenticatedTransport";
import { conversationIdSchema } from "../../features/assistant/assistantTypes";
import { groupIdSchema } from "../../features/groups/groupContracts";

type Request = (path: string, options?: RequestOptions) => Promise<unknown>;

/** A completed agent reply in a NyxBot chat or a group transcript. */
export type ReplyReportTarget = {
  kind: "conversation" | "group";
  id: string;
  messageId: string;
};

export const REPORT_REASONS = [
  "Offensive or abusive",
  "Harmful or unsafe",
  "Inaccurate or misleading",
  "Something else",
] as const;
export type ReportReason = (typeof REPORT_REASONS)[number];

const REPORT_PREFIX = "Report: ";
// The feedback endpoint stores at most 1,000 characters per comment.
const MAX_COMMENT = 1000;

export function reportComment(reason: ReportReason, details?: string) {
  const extra = details?.trim();
  return `${REPORT_PREFIX}${reason}${extra ? `. ${extra}` : ""}`.slice(
    0,
    MAX_COMMENT,
  );
}

/** Reports ride the existing reply feedback: a down vote tagged "Report: …". */
export function isReported(
  feedback: { vote?: string; comment?: string | null } | null | undefined,
) {
  return (
    feedback?.vote === "down" &&
    (feedback.comment ?? "").startsWith(REPORT_PREFIX)
  );
}

function feedbackPath(target: ReplyReportTarget) {
  const parent =
    target.kind === "conversation"
      ? `conversations/${conversationIdSchema.parse(target.id)}`
      : `groups/${groupIdSchema.parse(target.id)}`;
  return `/assistant/nyxagent/${parent}/messages/${encodeURIComponent(target.messageId)}/feedback`;
}

export function createReplyReportApi(request: Request) {
  return {
    async report(
      target: ReplyReportTarget,
      reason: ReportReason,
      details?: string,
    ) {
      await request(feedbackPath(target), {
        method: "PUT",
        body: { vote: "down", comment: reportComment(reason, details) },
      });
    },
  };
}

import { channelPlatformName } from "../../components/workspace/workspaceChatListModel";

// Event classification mirrors origin/main nyxbot-labels.ts.
// "NyxID events (authored by NyxID; ...):" -- the parenthetical has changed
// wording over time, so any NyxID events header is stripped.
const EVENT_HEADER = /^NyxID events[^:\n]*:\s*/;

/**
 * The individual notices of a server-authored `event` message. The server
 * writes one `- ` line per event, but an event's text may itself span lines
 * (a specialist's multi-line reply excerpt), so a new notice starts only at a
 * top-level `- ` line outside an open quote; the server swaps double quotes
 * inside quoted text for single ones, so an odd count means still inside.
 * Text without the events header (an automation's instruction) is one notice.
 */
export function eventNotices(text: string): string[] {
  if (!EVENT_HEADER.test(text)) return [text.trim()];
  const notices: string[] = [];
  for (const line of text.replace(EVENT_HEADER, "").split("\n")) {
    const current = notices.at(-1);
    const insideQuote =
      current !== undefined && (current.match(/"/g)?.length ?? 0) % 2 === 1;
    if (/^- /.test(line) && !insideQuote) notices.push(line.slice(2).trim());
    else if (current === undefined)
      notices.push(line.replace(/^\s*-\s+/, "").trim());
    else notices[notices.length - 1] = `${current}\n${line}`;
  }
  const kept = notices.map((notice) => notice.trim()).filter(Boolean);
  return kept.length ? kept : [text.trim()];
}

export type EventNoticeKind =
  | "reply"
  | "request"
  | "decision"
  | "approval"
  | "connection"
  | "channel"
  | "message"
  | "task"
  | "notice";
export type EventNoticeTone = "neutral" | "success" | "warning" | "destructive";

/** One NyxID notice, summarised for a compact activity card. */
export interface EventNotice {
  readonly kind: EventNoticeKind;
  readonly tone: EventNoticeTone;
  /** One line: who did what. */
  readonly summary: string;
  /** The rest (a reply excerpt, a reason), shown when the card is expanded. */
  readonly detail?: string;
  /** The specialist the notice is about, when it names one. */
  readonly agent?: string;
}

function firstSentence(text: string): string {
  const flat = text.replace(/\s+/g, " ").trim();
  const end = flat.search(/[.!?](\s|$)/);
  const sentence = end > 0 ? flat.slice(0, end) : flat;
  return sentence.length > 120 ? `${sentence.slice(0, 119)}…` : sentence;
}

/**
 * Classify a server-authored notice (see the backend `assistant_team` and
 * `nyxbot` notifiers) into a compact card. Unknown shapes fall back to their
 * first sentence, so new notices still render.
 */
export function describeEventNotice(text: string): EventNotice {
  const notice = text.trim();
  if (
    notice.startsWith("Skill ") &&
    (notice.includes("published and attached") ||
      notice.includes("requires a corrected package"))
  ) {
    return {
      kind: "notice",
      tone: notice.includes("published and attached") ? "success" : "warning",
      summary: "Skill publication",
      detail: notice,
    };
  }
  // The server swaps double quotes in quoted text for single ones.
  let match =
    /^Specialist (\S+) (replied|was stopped|failed \(([^)]*)\))\. Reply excerpt: "([^"]*)"/.exec(
      notice,
    );
  if (match) {
    const [, agent = "", status = ""] = match;
    const failed = status.startsWith("failed");
    return {
      kind: "reply",
      tone: failed
        ? "destructive"
        : status === "replied"
          ? "neutral"
          : "warning",
      summary: `${agent} ${failed ? "failed" : status}`,
      detail: match[4]?.trim() || undefined,
      agent,
    };
  }
  match =
    /^Specialist (\S+) requests (?:service (\S+)|read-only account access) \(request_id [^)]*\)\.\s*([\s\S]*?)(?:\s*Decide with [\s\S]*)?$/.exec(
      notice,
    );
  if (match) {
    const [, agent = "", service] = match;
    return {
      kind: "request",
      tone: "warning",
      summary: service
        ? `${agent} asks to use ${service}`
        : `${agent} asks to read your account`,
      detail: match[3]?.trim() || undefined,
      agent,
    };
  }
  match =
    /^(NyxBot|The user) (allowed|denied) your request for (?:service (\S+)|read-only account access)\.(?:\s*Reason: "([^"]*)")?/.exec(
      notice,
    );
  if (match) {
    const [, by = "", verdict = "", service, reason] = match;
    return {
      kind: "decision",
      tone: verdict === "allowed" ? "success" : "destructive",
      summary: `${by === "NyxBot" ? "NyxBot" : "You"} ${verdict} ${service ?? "read-only account access"}`,
      detail: reason?.trim() || undefined,
    };
  }
  match =
    /^The user (approved|rejected) the (\S+) approval request \(([\s\S]*?)\)(?: via (\S+))?\./.exec(
      notice,
    );
  if (match) {
    const [, verdict = "", service = "", description, via] = match;
    return {
      kind: "approval",
      tone: verdict === "approved" ? "success" : "destructive",
      summary: `You ${verdict} the ${service} request${via ? ` via ${channelPlatformName(via)}` : ""}`,
      detail: description?.trim() || undefined,
    };
  }
  match =
    /^The user (finished|declined) connecting (\S+?)(?: \(connect_link_id [^)]*\))?\./.exec(
      notice,
    );
  if (match) {
    const [, verdict = "", service = ""] = match;
    return {
      kind: "connection",
      tone: verdict === "finished" ? "success" : "neutral",
      summary: `${verdict === "finished" ? "Finished" : "Declined"} connecting ${service}`,
    };
  }
  match =
    /^The (\S+) channel bot (\S+) (?:the user just created is now linked to (\S+)\.|was created but could not be linked)/.exec(
      notice,
    );
  if (match) {
    const [, platform = "", label = "", agent] = match;
    const name = channelPlatformName(platform);
    return {
      kind: "channel",
      tone: agent ? "success" : "destructive",
      summary: agent
        ? `${name} bot ${label} linked to ${agent}`
        : `${name} bot ${label} could not be linked`,
    };
  }
  match = /^The user verified their (\S+) account on channel bot (\S+);/.exec(
    notice,
  );
  if (match) {
    const [, platform = "", label = ""] = match;
    return {
      kind: "channel",
      tone: "success",
      summary: `Verified your ${channelPlatformName(platform)} account on ${label}`,
    };
  }
  match =
    /^The owner sent another (\S+) message while you were working\.[\s\S]*?: "([^"]*)"/.exec(
      notice,
    );
  if (match) {
    return {
      kind: "message",
      tone: "neutral",
      summary: `New ${channelPlatformName(match[1] ?? "")} message`,
      detail: match[2]?.trim() || undefined,
    };
  }
  match = /^Asynchronous service operation finished: ([^.]+)\./.exec(notice);
  if (match) {
    const outcome = match[1]?.trim() ?? "";
    const ok = outcome === "done";
    return {
      kind: "task",
      tone: ok ? "success" : "warning",
      summary: ok
        ? "Background task finished"
        : `Background task ${outcome.replace(/_/g, " ")}`,
    };
  }
  const summary = firstSentence(notice);
  return {
    kind: "notice",
    tone: "neutral",
    summary,
    detail:
      summary.length < notice.replace(/\s+/g, " ").trim().length
        ? notice
        : undefined,
  };
}

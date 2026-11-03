/**
 * Recipient pills for a group composer. The shown recipients are the agents
 * the draft @mentions plus those picked by hand, minus those removed by hand;
 * with neither, the group's default (the sender's sticky last recipients,
 * else the lead). Mentioning a removed agent again brings it back. The
 * selection is sent as explicit IDs so a stale preview cannot silently route
 * to another member. Ambiguous bare handles still require server validation.
 */
import { mentionedNames } from "@/lib/assistant/nyxbot-mentions";

export interface RecipientMember {
  readonly id: string;
  readonly name: string;
  readonly destroyed?: boolean;
  readonly unavailable?: boolean;
}

export interface RecipientSelection {
  /** Agents the current draft @mentions, in order. */
  readonly mentioned: readonly string[];
  readonly added: readonly string[];
  readonly removed: readonly string[];
}

export const NO_RECIPIENT_EDITS: RecipientSelection = {
  mentioned: [],
  added: [],
  removed: [],
};

function live(members: readonly RecipientMember[]): RecipientMember[] {
  return members.filter((member) => !member.destroyed && !member.unavailable);
}

function unique(ids: readonly string[]): string[] {
  return ids.filter((id, index) => ids.indexOf(id) === index);
}

/** Live members the text @mentions, by the server's rule. */
export function mentionedIds(
  text: string,
  members: readonly RecipientMember[],
): string[] {
  const candidates = live(members);
  return mentionedNames(
    text,
    [...members.map((member) => member.id), ...members.map((member) => member.name)],
  ).flatMap((name) => {
    const exact = members.find((row) => row.id.toLowerCase() === name.toLowerCase());
    const named = members.filter((row) => row.name.toLowerCase() === name.toLowerCase());
    const member = exact ?? (named.length === 1 ? named[0] : undefined);
    return member && candidates.includes(member) ? [member.id] : [];
  });
}

/** Use an exact ID in the draft when a handle is ambiguous, including unavailable namesakes. */
export function memberMention(member: RecipientMember, members: readonly RecipientMember[]): string {
  return members.filter((row) => row.name.toLowerCase() === member.name.toLowerCase()).length > 1
    || members.some((row) => row.id.toLowerCase() === member.name.toLowerCase())
    ? member.id : member.name;
}

/** The draft changed: a newly typed mention undoes that agent's removal. */
export function withDraft(
  selection: RecipientSelection,
  text: string,
  members: readonly RecipientMember[],
): RecipientSelection {
  const mentioned = mentionedIds(text, members);
  const fresh = mentioned.filter((id) => !selection.mentioned.includes(id));
  return {
    ...selection,
    mentioned,
    removed: selection.removed.filter((id) => !fresh.includes(id)),
  };
}

export function withAdded(
  selection: RecipientSelection,
  id: string,
): RecipientSelection {
  return {
    ...selection,
    added: unique([...selection.added, id]),
    removed: selection.removed.filter((value) => value !== id),
  };
}

export function withRemoved(
  selection: RecipientSelection,
  id: string,
): RecipientSelection {
  return {
    ...selection,
    added: selection.added.filter((value) => value !== id),
    removed: unique([...selection.removed, id]),
  };
}

export interface ResolvedRecipients {
  /** Who the message goes to, as the pills show it. */
  readonly ids: string[];
  /** Whether a hand edit overrides mentions in the text. */
  readonly edited: boolean;
}

export function resolveRecipients(
  selection: RecipientSelection,
  defaults: readonly string[],
  members: readonly RecipientMember[],
): ResolvedRecipients {
  const liveIds = live(members).map((member) => member.id);
  const keep = (id: string) =>
    liveIds.includes(id) && !selection.removed.includes(id);
  const chosen = unique([...selection.mentioned, ...selection.added]);
  const ids = chosen.length
    ? chosen.filter(keep)
    : unique(defaults).filter(keep);
  const edited = [...selection.added, ...selection.removed].some((id) =>
    liveIds.includes(id),
  );
  return { ids, edited };
}

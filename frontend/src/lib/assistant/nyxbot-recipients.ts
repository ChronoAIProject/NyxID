/**
 * Recipient pills for a group composer. The shown recipients are the agents
 * the draft @mentions plus those picked by hand, minus those removed by hand;
 * with neither, the group's default (the sender's sticky last recipients,
 * else the lead). Mentioning a removed agent again brings it back. The
 * selection is sent explicitly only after a hand edit, so unedited messages
 * route on the server exactly as previewed.
 */
import { mentionedNames } from "@/lib/assistant/nyxbot-mentions";

export interface RecipientMember {
  readonly id: string;
  readonly name: string;
  readonly destroyed?: boolean;
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
  return members.filter((member) => !member.destroyed);
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
    candidates.map((member) => member.name),
  ).flatMap((name) => {
    const member = candidates.find((row) => row.name === name);
    return member ? [member.id] : [];
  });
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
  /** Whether the pills were edited by hand (then `ids` are sent explicitly). */
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

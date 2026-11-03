import type { AssistantAgent } from "../agents/agentContracts";
import type {
  Group,
  GroupMessage,
  GroupMessagesPage,
  GroupOrganizationMember,
  GroupPendingAction,
} from "./groupContracts";

export interface RecipientMember {
  readonly id: string;
  readonly name: string;
  readonly destroyed?: boolean;
  readonly unavailable?: boolean;
}
export interface RecipientSelection {
  readonly mentioned: readonly string[];
  readonly added: readonly string[];
  readonly removed: readonly string[];
}
export const NO_RECIPIENT_EDITS: RecipientSelection = {
  mentioned: [],
  added: [],
  removed: [],
};
const unique = (ids: readonly string[]) => [...new Set(ids)];

/** A draft's already resolved handle keeps its identity across roster edits. */
function boundMentionNames(members: readonly RecipientMember[]) {
  const ids = new Map<string, Set<string>>();
  for (const member of members) {
    const name = member.name.toLowerCase();
    const values = ids.get(name) ?? new Set<string>();
    values.add(member.id);
    ids.set(name, values);
  }
  return new Map(
    [...ids].flatMap(([name, values]) =>
      values.size === 1 ? [[name, [...values][0]!] as const] : [],
    ),
  );
}

/** An unavailable namesake still makes a bare handle ambiguous. */
export function ambiguousGroupMentions(
  text: string,
  members: readonly RecipientMember[],
  bindings: readonly RecipientMember[] = [],
): string[] {
  const bound = boundMentionNames(bindings);
  const names = new Map<string, number>();
  for (const row of members)
    names.set(
      row.name.toLowerCase(),
      (names.get(row.name.toLowerCase()) ?? 0) + 1,
    );
  return unique(
    [...text.matchAll(/(?:^|[^\p{L}\p{N}_.-])@([A-Za-z0-9_-]+)/gu)].flatMap(
      (match) =>
        !bound.has(match[1]!.toLowerCase()) &&
        (names.get(match[1]!.toLowerCase()) ?? 0) > 1
          ? [match[1]!]
          : [],
    ),
  );
}

/** Same word boundary and handle matching as the group router. */
export function mentionedIds(
  text: string,
  members: readonly RecipientMember[],
  includeUnavailable = false,
  bindings: readonly RecipientMember[] = [],
): string[] {
  return unique(
    resolveMentionBindings(text, members, includeUnavailable, bindings).map(
      (member) => member.id,
    ),
  );
}

/** Only handles still present in this edit retain a prior resolved identity. */
export function groupMentionBindings(
  text: string,
  members: readonly RecipientMember[],
  bindings: readonly RecipientMember[] = [],
): RecipientMember[] {
  return resolveMentionBindings(text, members, true, bindings);
}

function resolveMentionBindings(
  text: string,
  members: readonly RecipientMember[],
  includeUnavailable: boolean,
  bindings: readonly RecipientMember[],
): RecipientMember[] {
  const names = new Map(
    members
      .filter(
        (member) =>
          (includeUnavailable || (!member.destroyed && !member.unavailable)) &&
          members.filter(
            (row) => row.name.toLowerCase() === member.name.toLowerCase(),
          ).length === 1,
      )
      .map((member) => [member.name.toLowerCase(), member.id]),
  );
  for (const [name, id] of boundMentionNames(bindings)) names.set(name, id);
  for (const member of members.filter(
    (row) => includeUnavailable || (!row.destroyed && !row.unavailable),
  ))
    names.set(member.id.toLowerCase(), member.id);
  const found: RecipientMember[] = [];
  for (
    let index = text.indexOf("@");
    index >= 0;
    index = text.indexOf("@", index + 1)
  ) {
    const before = index > 0 ? (text[index - 1] ?? "") : "";
    if (before && /[\p{L}\p{N}_.-]/u.test(before)) continue;
    const token = /^[A-Za-z0-9_-]+/.exec(text.slice(index + 1))?.[0];
    const id = token
      ? includeUnavailable &&
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
          token,
        )
        ? token.toLowerCase()
        : names.get(token.toLowerCase())
      : undefined;
    if (id && !found.some((row) => row.name === token!.toLowerCase()))
      found.push({ id, name: token!.toLowerCase() });
  }
  return found;
}
export function withDraft(
  selection: RecipientSelection,
  text: string,
  members: readonly RecipientMember[],
  bindings: readonly RecipientMember[] = [],
): RecipientSelection {
  // An explicit mention remains a selection when its share is revoked. Filtering
  // it out here would make resolveRecipients silently choose the sticky default.
  const mentioned = mentionedIds(
    text,
    members,
    true,
    bindings.filter((member) => selection.mentioned.includes(member.id)),
  );
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
export function resolveRecipients(
  selection: RecipientSelection,
  defaults: readonly string[],
  members: readonly RecipientMember[],
): { ids: string[]; edited: boolean } {
  const liveIds = members
    .filter((member) => !member.destroyed && !member.unavailable)
    .map((member) => member.id);
  const keep = (id: string) =>
    liveIds.includes(id) && !selection.removed.includes(id);
  const chosen = unique([...selection.mentioned, ...selection.added]);
  return {
    ids: (chosen.length ? chosen : unique(defaults)).filter(keep),
    edited: [...selection.added, ...selection.removed].some((id) =>
      liveIds.includes(id),
    ),
  };
}

/** Missing/revoked explicit targets need a new human choice, never a default. */
export function unavailableGroupRecipients(
  selection: RecipientSelection,
  members: readonly RecipientMember[],
): string[] {
  return unique([...selection.mentioned, ...selection.added]).filter(
    (id) =>
      !selection.removed.includes(id) &&
      !members.some(
        (member) =>
          member.id === id && !member.destroyed && !member.unavailable,
      ),
  );
}

/** Newer rows replace overlapping sequence numbers, including server updates. */
export function mergeGroupMessages(
  older: readonly GroupMessage[],
  newer: readonly GroupMessage[],
): GroupMessage[] {
  return [
    ...new Map(
      [...older, ...newer].map((message) => [message.seq, message]),
    ).values(),
  ].sort((a, b) => a.seq - b.seq);
}
export function mergeGroupPage(
  current: GroupMessagesPage | undefined,
  incoming: GroupMessagesPage,
  mode: "refresh" | "older" = "refresh",
): GroupMessagesPage {
  if (!current)
    return { ...incoming, messages: mergeGroupMessages([], incoming.messages) };
  if (current.group.id !== incoming.group.id)
    throw new Error("Cannot combine messages from different crews.");
  if (mode === "older")
    return {
      ...current,
      messages: mergeGroupMessages(incoming.messages, current.messages),
      before_seq: incoming.before_seq,
    };
  const oldest = incoming.messages[0]?.seq ?? Infinity;
  const kept = current.messages.filter((message) => message.seq < oldest);
  return {
    ...incoming,
    messages: mergeGroupMessages(kept, incoming.messages),
    before_seq: kept.length ? current.before_seq : incoming.before_seq,
  };
}

export function canManageGroup(group: Group): boolean {
  if (group.collaboration) return group.collaboration.can_manage;
  return (
    group.owner?.type !== "org" ||
    group.your_role === "creator" ||
    group.your_role === "admin"
  );
}
export function canLeaveGroup(group: Group): boolean {
  if (group.collaboration) return true;
  return group.owner?.type === "org" && group.your_role !== undefined;
}
export const groupCanManage = canManageGroup;
export const groupCanLeave = canLeaveGroup;

/** Same compact live-status wording as the web crew transcript. */
export function groupWorkingLabel(names: readonly string[]): string {
  if (!names.length) return "";
  if (names.length === 1) return `${names[0]!} is working…`;
  if (names.length === 2) return `${names[0]!} and ${names[1]!} are working…`;
  return `${names[0]!}, ${names[1]!} and ${String(names.length - 2)} more are working…`;
}

/** Personal groups may use shared org agents that the caller can use. */
export function eligibleGroupAgents(
  agents: readonly AssistantAgent[],
  orgId?: string,
): AssistantAgent[] {
  return agents
    .filter(
      (agent) =>
        agent.status !== "destroyed" &&
        agent.can_use !== false &&
        (!orgId || (agent.owner_kind === "org" && agent.owner_id === orgId)),
    )
    .sort((a, b) => Number(b.kind === "nyxbot") - Number(a.kind === "nyxbot"));
}
export function eligibleGroupPeople(
  members: readonly GroupOrganizationMember[],
): GroupOrganizationMember[] {
  return members.filter(
    (member) => !member.revoked_at && ["admin", "member"].includes(member.role),
  );
}

export function groupPendingOwned(
  action: GroupPendingAction,
  actorId?: string,
  orgGroup = false,
  now = Date.now(),
): boolean {
  const expires = Date.parse(action.expires_at);
  if (!Number.isFinite(expires) || expires <= now) return false;
  if (!orgGroup && !action.triggering_person) return true;
  return !!actorId && action.triggering_person?.id === actorId;
}
export function groupCanDecide(
  action: GroupPendingAction,
  actorId?: string,
  orgGroup = false,
  now = Date.now(),
): boolean {
  return (
    !action.authored_skill &&
    groupPendingOwned(action, actorId, orgGroup, now) &&
    (orgGroup ? action.can_decide === true : !!action.confirm_phrase)
  );
}
/** Never derive native navigation from the server-supplied review_url. */
export function groupPendingReview(
  action: GroupPendingAction,
  actorId?: string,
  orgGroup = false,
  now = Date.now(),
):
  | { kind: "assistant"; conversationId: string; acknowledgementId: string }
  | undefined {
  if (
    !action.authored_skill ||
    !action.acknowledgement_id ||
    !groupPendingOwned(action, actorId, orgGroup, now)
  )
    return undefined;
  return {
    kind: "assistant",
    conversationId: action.conversation_id,
    acknowledgementId: action.acknowledgement_id,
  };
}

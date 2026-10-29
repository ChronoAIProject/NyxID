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

/** Same word boundary and handle matching as the group router. */
export function mentionedIds(
  text: string,
  members: readonly RecipientMember[],
): string[] {
  const names = new Map(
    members
      .filter((member) => !member.destroyed)
      .map((member) => [member.name.toLowerCase(), member.id]),
  );
  const found: string[] = [];
  for (
    let index = text.indexOf("@");
    index >= 0;
    index = text.indexOf("@", index + 1)
  ) {
    const before = index > 0 ? (text[index - 1] ?? "") : "";
    if (before && /[\p{L}\p{N}_.-]/u.test(before)) continue;
    const token = /^[A-Za-z0-9_-]+/.exec(text.slice(index + 1))?.[0];
    const id = token ? names.get(token.toLowerCase()) : undefined;
    if (id && !found.includes(id)) found.push(id);
  }
  return found;
}
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
export function resolveRecipients(
  selection: RecipientSelection,
  defaults: readonly string[],
  members: readonly RecipientMember[],
): { ids: string[]; edited: boolean } {
  const liveIds = members
    .filter((member) => !member.destroyed)
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
  return (
    group.owner?.type !== "org" ||
    group.your_role === "creator" ||
    group.your_role === "admin"
  );
}
export function canLeaveGroup(group: Group): boolean {
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

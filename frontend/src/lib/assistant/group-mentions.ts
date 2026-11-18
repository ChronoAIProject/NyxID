import { mentionTokens } from "./nyxbot-mentions";
import { personAlias, type MentionPerson } from "./collaboration-mentions";

export interface GroupMentionMember {
  readonly id: string;
  readonly name: string;
  readonly display_name?: string | null;
  readonly mention_handle?: string;
}
export interface InlineMention { readonly handle: string; readonly label: string }
const UUID = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i;

/** Rendering only. Persisted text and recipient IDs remain authoritative for routing. */
export function groupMentionProjection(text: string, members: readonly GroupMentionMember[], people: readonly MentionPerson[]) {
  const targets = [
    ...members.map((member, i) => ({ id: member.id, handle: member.mention_handle ?? `member-${i + 1}`, alias: member.name, label: member.display_name?.trim() || member.name })),
    ...people.map((person, i) => ({ id: person.id, handle: person.handle ?? `person-${i + 1}`, alias: personAlias(person), label: person.display_name })),
  ];
  const mentions: InlineMention[] = targets.flatMap((target) => [
    { handle: target.handle, label: target.label },
    ...(targets.filter((p) => p.alias.toLowerCase() === target.alias.toLowerCase()).length === 1 ? [{ handle: target.alias, label: target.label }] : []),
  ]);
  mentions.push({ handle: "unknown-member", label: "unknown member" });
  let safeText = text;
  for (const token of mentionTokens(text).reverse()) {
    if (!UUID.test(token.name)) continue;
    const target = targets.find((p) => p.id.toLowerCase() === token.name.toLowerCase());
    safeText = safeText.slice(0, token.start) + `@${target?.handle ?? "unknown-member"}` + safeText.slice(token.start + token.name.length + 1);
  }
  let copyText = safeText;
  for (const token of mentionTokens(safeText).reverse()) {
    const mention = mentions.find((m) => m.handle.toLowerCase() === token.name.toLowerCase());
    if (mention && (targets.some((t) => t.handle.toLowerCase() === token.name.toLowerCase()) || token.name === "unknown-member")) copyText = copyText.slice(0, token.start) + `@${mention.label}` + copyText.slice(token.start + token.name.length + 1);
  }
  return { text: safeText, mentions, copyText };
}

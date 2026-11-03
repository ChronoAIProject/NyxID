import { mentionTokens } from "./nyxbot-mentions";

export interface MentionPerson {
  readonly id: string;
  readonly display_name: string;
  readonly handle?: string;
}
export function personAlias(person: MentionPerson): string {
  return person.display_name.trim().split(/\s+/).join("-").toLowerCase();
}
/** Preview only; the server resolves again against transactional live membership. */
export function peopleMentions(
  text: string,
  people: readonly MentionPerson[],
  agents: readonly { id: string; name: string; mention_handle?: string }[],
  enabled = true,
) {
  if (!enabled) return { ids: [], hasPeople: false, ambiguous: false, agentText: text };
  const ids: string[] = [];
  let hasPeople = false;
  let ambiguous = false;
  let agentText = text;
  const hidden: { start: number; end: number }[] = [];
  for (const token of mentionTokens(text)) {
    const name = token.name.toLowerCase();
    if (agents.some((a) => a.id.toLowerCase() === name || a.mention_handle?.toLowerCase() === name)) continue;
    const exact = people.filter((p) => (/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(name) && p.id.toLowerCase() === name) || p.handle?.toLowerCase() === name);
    const named = people.filter((p) => personAlias(p) === name);
    const candidates = exact.length ? exact : named;
    if (!candidates.length && !/^(?:person|agent)-[0-9a-f]{16}$/.test(name) && !/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(name)) continue;
    hasPeople = true;
    const collision = candidates.length > 1 || (!exact.length && named.length > 0 && agents.some((a) => a.name.toLowerCase() === name));
    ambiguous ||= collision;
    if (!collision && candidates.length === 1 && !ids.includes(candidates[0]!.id)) ids.push(candidates[0]!.id);
    hidden.push({ start: token.start, end: token.start + token.name.length + 1 });
  }
  for (const { start, end } of hidden.reverse()) agentText = agentText.slice(0, start) + " ".repeat(end - start) + agentText.slice(end);
  return { ids, hasPeople, ambiguous, agentText };
}

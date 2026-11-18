import { describe, expect, it } from "vitest";
import { groupMentionProjection } from "./group-mentions";
import { mentionedIds } from "./nyxbot-recipients";
import { peopleMentions } from "./collaboration-mentions";

const agent = { id: "b775452f-441f-4db9-a14d-accb96371ff2", name: "office", display_name: "ChronoAI Office Agent", mention_handle: "agent-0123456789abcdef" };
const person = { id: "00000000-0000-4000-8000-000000000001", display_name: "calvin tan", handle: "person-0123456789abcdef" };
describe("public group mention projection", () => {
  it("renders UUID mentions with public handles while exact IDs still route", () => {
    const text = `@${agent.id} and @${person.id}`;
    const view = groupMentionProjection(text, [agent], [person]);
    expect(view.text).toBe(`@${agent.mention_handle} and @${person.handle}`);
    expect(view.copyText).toBe("@ChronoAI Office Agent and @calvin tan");
    expect(mentionedIds(`@${agent.id}`, [agent])).toEqual([agent.id]);
    expect(mentionedIds(`@${agent.mention_handle}`, [agent])).toEqual([agent.id]);
    expect(peopleMentions(`@${person.id}`, [person], [agent]).ids).toEqual([person.id]);
  });
  it("renders unknown UUIDs neutrally and never routes to a namesake or fallback", () => {
    const id = "00000000-0000-4000-8000-000000000009";
    const text = `@${id}`;
    expect(groupMentionProjection(text, [agent], [person]).copyText).toBe("@unknown member");
    expect(peopleMentions(text, [person], [agent])).toMatchObject({ hasPeople: true, ids: [] });
    expect(mentionedIds(peopleMentions(text, [person], [agent]).agentText, [agent])).toEqual([]);
  });
});

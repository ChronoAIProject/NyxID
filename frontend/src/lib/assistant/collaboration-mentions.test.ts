import { describe, expect, it } from "vitest";
import { peopleMentions } from "./collaboration-mentions";

const people = [
  { id: "alice", display_name: "Alice", handle: "person-0123456789abcdef" },
  { id: "calvin", display_name: "calvin tan", handle: "person-abcdef0123456789" },
];
describe("collaboration people mentions", () => {
  it("recognizes exact handles and bare names without passing them to agent routing", () => {
    const result = peopleMentions("@person-0123456789abcdef and @calvin-tan @helper", people, [{ id: "agent", name: "helper" }]);
    expect(result.ids).toEqual(["alice", "calvin"]);
    expect(result.agentText.trim()).toContain("@helper");
    expect(result.agentText).not.toContain("@person");
    expect(result.ambiguous).toBe(false);
  });
  it("rejects human and agent namesakes but exact handles remain deterministic", () => {
    expect(peopleMentions("@Alice", [...people, { id: "other", display_name: "Alice", handle: "person-1111111111111111" }], []).ambiguous).toBe(true);
    expect(peopleMentions("@Alice", people, [{ id: "agent", name: "alice" }]).ambiguous).toBe(true);
    expect(peopleMentions("@person-0123456789abcdef", people, [{ id: "agent", name: "alice" }]).ids).toEqual(["alice"]);
  });
  it("preserves stale person intent without targeting a nonparticipant or affecting legacy crews", () => {
    expect(peopleMentions("@person-0000000000000000", people, [])).toMatchObject({ ids: [], hasPeople: true });
    expect(peopleMentions("@person-0000000000000000", [], [], false)).toMatchObject({ ids: [], hasPeople: false, agentText: "@person-0000000000000000" });
  });
});

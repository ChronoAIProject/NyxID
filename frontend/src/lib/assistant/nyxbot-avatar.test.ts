import { describe, expect, it } from "vitest";
import { AGENT_AVATAR_TINTS, agentInitials, agentTintIndex } from "./nyxbot-avatar";

describe("agent avatars", () => {
  it("pick a stable tint per agent id", () => {
    const ids = Array.from({ length: 40 }, (_, index) => `agent-${String(index)}`);
    for (const id of ids) {
      const tint = agentTintIndex(id);
      expect(tint).toBe(agentTintIndex(id));
      expect(tint).toBeGreaterThanOrEqual(0);
      expect(tint).toBeLessThan(AGENT_AVATAR_TINTS.length);
    }
    // Different agents spread over the palette.
    expect(new Set(ids.map(agentTintIndex)).size).toBeGreaterThan(3);
  });

  it("uses only design tokens and never the destructive red", () => {
    for (const tint of AGENT_AVATAR_TINTS) expect(tint).not.toMatch(/destructive|red-/);
  });

  it("derives initials from the name", () => {
    expect(agentInitials("researcher")).toBe("RE");
    expect(agentInitials("release-notes")).toBe("RN");
    expect(agentInitials("a")).toBe("A");
    expect(agentInitials("")).toBe("?");
  });
});

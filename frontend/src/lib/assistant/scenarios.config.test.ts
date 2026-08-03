import { describe, expect, it } from "vitest";
import { compileScenarioSet } from "@/lib/assistant/scenario-engine";
import { compiledScenarios, flows, scenarios } from "./scenarios.config";

describe("assistant mock scenario config", () => {
  it("loads the complete initial scenario set", () => {
    expect(compiledScenarios.scenarios.map((entry) => entry.id)).toEqual([
      "connect-github",
      "github-issues",
      "github-issues-repo",
      "approval-demo",
      "error-demo",
    ]);
  });

  it("uses unique ids and reconstructable regular expressions", () => {
    const ids = scenarios.map((entry) => entry.id);
    expect(new Set(ids).size).toBe(ids.length);

    for (const entry of scenarios) {
      const reconstructed = new RegExp(
        entry.pattern.source,
        entry.pattern.flags,
      );
      expect(reconstructed.source).toBe(entry.pattern.source);
    }
  });

  it("resolves every flow and validates every action at config load", () => {
    expect(() => compileScenarioSet(scenarios, flows)).not.toThrow();
    expect(Object.keys(flows)).toEqual(["connect-github"]);
  });

  it("preserves first-match ordering for overlapping GitHub issue prompts", () => {
    const match = compiledScenarios.scenarios.find((entry) => {
      entry.pattern.lastIndex = 0;
      return entry.pattern.test("show github issues in acme/web");
    });

    expect(match?.id).toBe("github-issues");
  });
});

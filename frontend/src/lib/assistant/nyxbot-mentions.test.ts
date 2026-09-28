import { describe, expect, it } from "vitest";
import {
  activeMention,
  insertMention,
  mentionCandidates,
  mentionSegments,
  mentionedNames,
} from "./nyxbot-mentions";

describe("activeMention", () => {
  it("finds the mention being typed at the caret", () => {
    expect(activeMention("@", 1)).toEqual({ start: 0, query: "" });
    expect(activeMention("hi @res", 7)).toEqual({ start: 3, query: "res" });
    // The caret inside a name still edits that mention.
    expect(activeMention("hi @research", 7)).toEqual({ start: 3, query: "res" });
  });

  it("ignores @ inside words, after whitespace ends it, or with no @", () => {
    expect(activeMention("mail@res", 8)).toBeUndefined();
    expect(activeMention("@res ", 5)).toBeUndefined();
    expect(activeMention("plain", 5)).toBeUndefined();
  });
});

describe("mentionCandidates", () => {
  const members = [{ name: "NyxBot" }, { name: "researcher" }, { name: "release-notes" }];
  it("matches case-insensitively, prefix matches first", () => {
    expect(mentionCandidates(members, "").map((row) => row.name)).toEqual([
      "NyxBot",
      "researcher",
      "release-notes",
    ]);
    expect(mentionCandidates(members, "NYX").map((row) => row.name)).toEqual(["NyxBot"]);
    expect(mentionCandidates(members, "not").map((row) => row.name)).toEqual(["release-notes"]);
    expect(mentionCandidates(members, "re").map((row) => row.name)).toEqual([
      "researcher",
      "release-notes",
    ]);
  });
});

describe("mentionCandidates with display names", () => {
  it("finds an agent by its display name too", () => {
    const members = [
      { name: "writer", display_name: "Luna" },
      { name: "researcher", display_name: null },
    ];
    expect(mentionCandidates(members, "lu").map((row) => row.name)).toEqual(["writer"]);
    expect(mentionCandidates(members, "w").map((row) => row.name)).toEqual(["writer"]);
  });
});

describe("insertMention", () => {
  it("replaces the partial mention and leaves one space after it", () => {
    expect(insertMention("ask @res", { start: 4, query: "res" }, 8, "researcher")).toEqual({
      text: "ask @researcher ",
      caret: 16,
    });
    // Completing mid-text keeps what follows without doubling the space.
    expect(
      insertMention("@re please", { start: 0, query: "re" }, 3, "researcher"),
    ).toEqual({ text: "@researcher please", caret: 12 });
  });
});

describe("mentionedNames and mentionSegments", () => {
  const names = ["NyxBot", "researcher"];
  it("returns each member mentioned once, case-insensitively", () => {
    expect(mentionedNames("@RESEARCHER and @nyxbot, @researcher", names)).toEqual([
      "researcher",
      "NyxBot",
    ]);
    expect(mentionedNames("mail@researcher @stranger", names)).toEqual([]);
  });

  it("splits text into plain and member-mention runs", () => {
    expect(mentionSegments("hi @researcher and @nobody", names)).toEqual([
      { text: "hi ", mention: false },
      { text: "@researcher", mention: true },
      { text: " and @nobody", mention: false },
    ]);
  });
});

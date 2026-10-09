import { describe, expect, it } from "vitest";
import { agentGrant, rowLevel, type AccessServiceRow } from "./agent-grant";

const row = (id: string, requiredByApp = false): AccessServiceRow => ({
  id,
  name: id,
  secondary: "",
  orgName: null,
  requiredByApp,
});
const rows = [row("github"), row("gmail"), row("slack")];

describe("agentGrant", () => {
  it("reads every service and writes none by default", () => {
    expect(agentGrant(rows, "read", {})).toEqual({
      allowAll: true,
      readIds: ["github", "gmail", "slack"],
      writeMode: "none",
      writeIds: [],
    });
  });

  it("writes everywhere only when the default is read and write", () => {
    expect(agentGrant(rows, "write", {})).toMatchObject({
      allowAll: true,
      writeMode: "all",
    });
  });

  it("lists write services chosen per row", () => {
    expect(agentGrant(rows, "read", { gmail: "write" })).toMatchObject({
      allowAll: true,
      writeMode: "selected",
      writeIds: ["gmail"],
    });
  });

  it("drops to an explicit list once any service is off", () => {
    expect(agentGrant(rows, "read", { slack: "off" })).toEqual({
      allowAll: false,
      readIds: ["github", "gmail"],
      writeMode: "none",
      writeIds: [],
    });
  });

  it("is not a write-all grant when one row is narrowed", () => {
    expect(agentGrant(rows, "write", { github: "read" })).toMatchObject({
      allowAll: true,
      writeMode: "selected",
      writeIds: ["gmail", "slack"],
    });
  });

  it("grants nothing when every service is off", () => {
    expect(agentGrant(rows, "off", {})).toEqual({
      allowAll: false,
      readIds: [],
      writeMode: "none",
      writeIds: [],
    });
  });
});

describe("rowLevel", () => {
  it("keeps services the app requested readable", () => {
    expect(rowLevel(row("github", true), "off", {})).toBe("read");
    expect(rowLevel(row("github", true), "write", {})).toBe("write");
  });
});

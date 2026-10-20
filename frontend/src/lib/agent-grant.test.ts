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
  it("grants nothing when every service is off", () => {
    expect(agentGrant(rows, "off", {})).toEqual({
      allowAll: false,
      serviceIds: [],
    });
  });

  it("allows future services only with an explicit all-services choice", () => {
    expect(agentGrant(rows, "allow", {})).toEqual({
      allowAll: true,
      serviceIds: ["github", "gmail", "slack"],
    });
    expect(agentGrant([], "off", {})).toEqual({
      allowAll: false,
      serviceIds: [],
    });
    expect(agentGrant([], "allow", {})).toEqual({
      allowAll: true,
      serviceIds: [],
    });
  });

  it("limits a per-service choice to that service", () => {
    expect(agentGrant(rows, "off", { gmail: "allow" })).toEqual({
      allowAll: false,
      serviceIds: ["gmail"],
    });
  });

  it("drops to an explicit list once any service is off", () => {
    expect(agentGrant(rows, "allow", { slack: "off" })).toEqual({
      allowAll: false,
      serviceIds: ["github", "gmail"],
    });
  });
});

describe("rowLevel", () => {
  it("keeps required services allowed", () => {
    expect(rowLevel(row("github", true), "off", {})).toBe("allow");
    expect(agentGrant([row("github", true)], "off", {})).toEqual({
      allowAll: false,
      serviceIds: ["github"],
    });
  });
});

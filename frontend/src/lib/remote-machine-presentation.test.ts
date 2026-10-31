import { expect, it } from "vitest";
import { remoteMachinePresentationSchema } from "@/schemas/machines";
import { remoteMachineReportedProfile } from "./remote-machine-presentation";
const report = {
  agent_version: "1.2.3",
  os: "linux",
  arch: "x86_64",
  roots: ["/workspace"],
  capabilities: ["shell", "files", "browser"],
  commands_isolated: true,
  browser_isolated: true,
  separation_support: true,
  owner_confirmation: "changes",
  allow_single_user_saved_logins: false,
  computer_mode: "standard",
  cua_version: "1",
  connected: false,
  desktop_ready: false,
  historical: true,
  configuration_current: false,
  last_reported_at: "2026-10-26T00:00:00Z",
};
it("parses the flat safe report and strips unapproved fields before caching", () => {
  const parsed = remoteMachinePresentationSchema.parse({
    ...report,
    token: "must-be-stripped",
    node: { secret: "also-stripped" },
  });
  expect(parsed).toEqual(report);
  expect(remoteMachineReportedProfile(parsed)).toMatchObject({
    shell: true,
    files: true,
    browser: true,
    computer: false,
    computer_ready: false,
    os: "linux",
  });
});
it("keeps an absent or unreported runtime unknown without inventing separation evidence", () => {
  expect(remoteMachineReportedProfile(undefined)).toBeUndefined();
  const parsed = remoteMachinePresentationSchema.parse({
    ...report,
    os: null,
    arch: null,
    capabilities: [],
    separation_support: null,
    agent_version: null,
  });
  expect(remoteMachineReportedProfile(parsed)).toBeUndefined();
  expect(
    remoteMachineReportedProfile(remoteMachinePresentationSchema.parse(report))
      ?.separation_evidence,
  ).toBeUndefined();
});

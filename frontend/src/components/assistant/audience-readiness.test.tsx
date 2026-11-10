import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { AudienceReadiness } from "./audience-readiness";
import { guestReadinessSettingsSchema } from "@/schemas/assistant-nyxagent";

afterEach(cleanup);
it("shows missing structural facts without blocking owner choice", () => {
  render(<AudienceReadiness configuration={{ instructions_present: false, persona_present: false, memory_notes: 0, skills: 0, service_grants: 0, account_read: false }} />);
  expect(screen.getByText(/Instructions: missing; persona: missing; memory notes: 0; skills: 0; service grants: 0/)).toBeTruthy();
  expect(screen.getByText(/Opening access is your choice/)).toBeTruthy();
  expect(screen.queryByRole("button")).toBeNull();
});
it("does not claim business readiness when configuration is present", () => {
  render(<AudienceReadiness configuration={{ instructions_present: true, persona_present: true, memory_notes: 2, skills: 1, service_grants: 3, account_read: true }} />);
  expect(screen.getByText(/Instructions: present/)).toBeTruthy();
  expect(screen.getByText(/do not confirm complete business information or a successful customer test/)).toBeTruthy();
});
it("keeps older servers compatible and never invents configuration", () => {
  const settings = guestReadinessSettingsSchema.parse({ status: "ready" });
  render(<AudienceReadiness configuration={settings.configuration} />);
  expect(screen.getByText("Agent configuration has not been checked.")).toBeTruthy();
});

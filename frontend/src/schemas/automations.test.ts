import { describe, expect, it } from "vitest";
import {
  automationDefaults,
  automationFormSchema,
  automationPreferencesSchema,
  automationRequest,
  scheduleSchema,
} from "./automations";

describe("automation forms", () => {
  const valid = () => ({
    ...automationDefaults("Asia/Singapore"),
    label: "Morning report",
    agent_id: "nyxbot",
    instruction: "Read GitHub notifications",
  });
  it("keeps cron timezone and owner result destination", () => {
    const form = valid();
    form.deliver_type = "chat";
    form.chat_id = "telegram-chat";
    const request = automationRequest(automationFormSchema.parse(form));
    expect(request.source).toBe("schedule");
    expect(request.schedule).toMatchObject({
      kind: "cron",
      timezone: "Asia/Singapore",
      expression: "0 8 * * 1-5",
    });
    expect(request.delivery).toMatchObject({
      type: "assistant",
      thread_policy: null,
      deliver_to: { type: "chat", chat_id: "telegram-chat" },
    });
  });
  it("validates instruction bytes, timezone, agent and destination", () => {
    for (const patch of [
      { instruction: "" },
      { instruction: "🐈".repeat(2049) },
      { timezone: "Mars/Olympus" },
      { agent_id: "" },
      { deliver_type: "chat", chat_id: "" },
      { expression: "* * *" },
      { max_runs: "0" },
    ]) {
      expect(
        automationFormSchema.safeParse({ ...valid(), ...patch }).success,
      ).toBe(false);
    }
  });
  it("builds interval, once and webhook specs and reads nullable backend options", () => {
    expect(
      automationRequest({
        ...valid(),
        kind: "every",
        amount: 5,
        unit: "minutes",
      }).schedule,
    ).toMatchObject({ kind: "every", amount: 5, unit: "minutes" });
    expect(
      automationRequest({
        ...valid(),
        kind: "at",
        at: "2026-11-01T08:00:00+08:00",
      }).schedule,
    ).toMatchObject({ kind: "at", at: "2026-11-01T00:00:00.000Z" });
    const hook = automationRequest({
      ...valid(),
      source: "webhook",
      verification_mode: "hmac",
    });
    expect(hook.schedule).toBeUndefined();
    expect(hook.verification).toEqual({
      mode: "hmac_sha256",
      header_name: "X-Hub-Signature-256",
    });
    expect(
      scheduleSchema.safeParse({
        kind: "cron",
        expression: "0 8 * * *",
        timezone: "UTC",
        start: null,
        end: null,
        max_runs: null,
        grace_seconds: null,
      }).success,
    ).toBe(true);
  });
  it("bounds owner preferences", () => {
    const preferences = {
      timezone: "Asia/Singapore",
      schedule_minimum_minutes: 5,
      trigger_runs_per_hour: 30,
      trigger_runs_per_day: 300,
    };
    expect(automationPreferencesSchema.safeParse(preferences).success).toBe(
      true,
    );
    for (const patch of [
      { schedule_minimum_minutes: 4 },
      { trigger_runs_per_hour: 301 },
      { trigger_runs_per_day: 3001 },
      { timezone: "" },
    ])
      expect(
        automationPreferencesSchema.safeParse({ ...preferences, ...patch })
          .success,
      ).toBe(false);
  });
});

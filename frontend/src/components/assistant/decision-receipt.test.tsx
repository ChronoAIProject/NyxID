import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DecisionReceipt } from "./decision-receipt";
import { appI18n } from "@/i18n";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";
const row: NyxAgentAcknowledgement = {
  id: "11111111-1111-4111-8111-111111111111", kind: "action", status: "allowed",
  summary: "Attach weekly-review 1.0 to NyxBot", decider: "user", decided_by: "user", reason: null,
  service_slug: null, service_name: null, tool_name: "nyxid__set_agent_skills",
  confirmation: { risk: "sensitive", access_mode: "nyxmode", reason: "access_mode" },
  created_at: "2026-10-14T00:00:00Z", decided_at: "2026-10-14T00:01:00Z", expires_at: "2026-10-14T00:10:00Z",
};
afterEach(async () => {
  cleanup();
  vi.useRealTimers();
  await appI18n.changeLanguage("en");
});
describe("DecisionReceipt", () => {
  it.each([
    ["allowed", "user", "You allowed"], ["denied", "user", "You denied"],
    ["allowed", "orchestrator", "NyxBot allowed"], ["denied", "orchestrator", "NyxBot denied"],
    ["allowed", "future_decider", "Allowed"], ["denied", "future_decider", "Denied"],
    ["allowed", null, "Allowed"],
    ["used", "user", "Used"], ["expired", "user", "Expired"],
  ] as const)("renders %s by %s as a durable status", (status, decided_by, label) => {
    render(<DecisionReceipt acknowledgement={{ ...row, status, decided_by }} />);
    expect(screen.getByRole("status")).toHaveTextContent(`${label} · ${row.summary}`);
    expect(screen.getByRole("status")).toHaveTextContent("Sensitive");
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
  it.each([
    ["access_mode", ""], ["webhook", "bypassed automation confirmation"],
    ["policy", "bypassed policy confirmation"], ["approval", "bypassed service approval"],
  ] as const)("explains the automatic %s bypass", (reason, phrase) => {
    render(<DecisionReceipt acknowledgement={{ ...row, decided_by: "access_mode", confirmation: { risk: "change", access_mode: "autopilot", reason } }} />);
    expect(screen.getByRole("status")).toHaveTextContent("Allowed automatically · Full access");
    if (phrase) expect(screen.getByRole("status")).toHaveTextContent(phrase);
    expect(screen.getByRole("status")).toHaveTextContent(row.summary);
  });
  it("uses the decided time and links an automatically published proposal", () => {
    vi.useFakeTimers(); vi.setSystemTime(new Date("2026-10-14T00:03:00Z"));
    render(<DecisionReceipt acknowledgement={{ ...row, decided_by: "access_mode", summary: "Skill weekly-review 1.0 published and attached to NyxBot", link: "https://nyxid.test/assistant?agent=a&proposal=p" }} />);
    expect(screen.getByRole("status")).toHaveTextContent("2m ago");
    expect(screen.getByText("· 2m ago")).toHaveAttribute("dateTime", row.decided_at);
    expect(screen.getByRole("link", { name: "Inspect or discard proposal" })).toHaveAttribute("href", "https://nyxid.test/assistant?agent=a&proposal=p");
  });
  it("explains client retirement without claiming expiry or success", () => {
    render(<DecisionReceipt acknowledgement={{ ...row, status: "pending", decided_by: null }} retired />);
    expect(screen.getByRole("status")).toHaveTextContent("No longer needed · Full access");
    expect(screen.getByRole("status")).not.toHaveTextContent("Expired");
    expect(screen.getByRole("status")).not.toHaveTextContent("You allowed");
  });
  it("leaves legacy receipt prose unchanged when no structured link exists", () => {
    render(<DecisionReceipt acknowledgement={{ ...row, summary: "Inspect or discard: https://nyxid.test/legacy" }} />);
    expect(screen.getByRole("status")).toHaveTextContent("Inspect or discard: https://nyxid.test/legacy");
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });
});

it.each([
  ["allowed", "user", "You allowed", "你已允许", "你已允許"],
  ["denied", "user", "You denied", "你已拒绝", "你已拒絕"],
  ["allowed", "orchestrator", "NyxBot allowed", "NyxBot 已允许", "NyxBot 已允許"],
  ["denied", "orchestrator", "NyxBot denied", "NyxBot 已拒绝", "NyxBot 已拒絕"],
  ["allowed", null, "Allowed", "已允许", "已允許"],
  ["denied", null, "Denied", "已拒绝", "已拒絕"],
  ["used", "user", "Used", "已使用", "已使用"],
  ["expired", "user", "Expired", "已过期", "已過期"],
  ["allowed", "access_mode", "Allowed automatically · Full access", "已自动允许 · 完全访问", "已自動允許 · 完整存取"],
] as const)("updates a mounted %s receipt by %s with the app language", async (status, decided_by, english, simplified, traditional) => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-14T00:03:00Z"));
  render(<DecisionReceipt acknowledgement={{ ...row, status, decided_by }} />);
  const receipt = screen.getByRole("status");
  expect(receipt).toHaveTextContent(`${english} · ${row.summary}`);
  expect(receipt.querySelector("time")).toHaveTextContent("2m ago");
  for (const [language, label, time] of [["zh-CN", simplified, "2 分钟前"], ["zh-TW", traditional, "2 分鐘前"], ["en", english, "2m ago"]]) {
    await act(async () => { await appI18n.changeLanguage(language); });
    expect(receipt).toHaveTextContent(`${label} · ${row.summary}`);
    expect(receipt.querySelector("time")).toHaveTextContent(time!);
  }
});

it.each([
  ["webhook", "automation confirmation", "自动化确认", "自動化確認"],
  ["policy", "policy confirmation", "策略确认", "原則確認"],
  ["org_connection", "organization instruction", "组织指示", "組織指示"],
  ["delegation", "delegation confirmation", "委派确认", "委派確認"],
  ["approval", "service approval", "服务审批", "服務核准"],
] as const)("translates the automatic %s reason on language changes", async (reason, english, simplified, traditional) => {
  render(<DecisionReceipt acknowledgement={{ ...row, decided_by: "access_mode", confirmation: { risk: "change", access_mode: "autopilot", reason } }} />);
  expect(screen.getByRole("status")).toHaveTextContent(`bypassed ${english}`);
  await act(async () => { await appI18n.changeLanguage("zh-CN"); });
  expect(screen.getByRole("status")).toHaveTextContent(`已跳过 ${simplified}`);
  await act(async () => { await appI18n.changeLanguage("zh-TW"); });
  expect(screen.getByRole("status")).toHaveTextContent(`已略過 ${traditional}`);
});

it("translates retirement and just-now timestamps without translating server prose", async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(row.created_at));
  render(<DecisionReceipt acknowledgement={{ ...row, status: "pending", decided_at: null }} retired />);
  expect(screen.getByRole("status")).toHaveTextContent("No longer needed · Full access");
  expect(screen.getByRole("status").querySelector("time")).toHaveTextContent("just now");
  await act(async () => { await appI18n.changeLanguage("zh-CN"); });
  expect(screen.getByRole("status")).toHaveTextContent(`不再需要 · 完全访问 · ${row.summary}`);
  expect(screen.getByRole("status").querySelector("time")).toHaveTextContent("刚刚");
});

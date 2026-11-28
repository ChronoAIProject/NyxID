import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { appI18n } from "@/i18n";
import type { TelegramNewRequest } from "@/schemas/telegram-new";
import { TelegramPromptStatus } from "./telegram-prompt-status";

const request: TelegramNewRequest = {
  id: "304c0a91-b6e0-48e4-8e1e-9fe39175bde7", status: "waiting_bot", revision: 2,
  label: "Support", owner_user_id: "owner", expires_at: "2099-10-27T02:00:00Z",
  telegram_bot_id: null, bot_username: null, channel_bot_id: null, auto_connect: true,
  can_open_creation_form: true,
};
const delivery = {
  attempt_id: "db234f25-1643-447d-8c72-04032b34db85", phase: "keyboard" as const,
  outcome: "accepted" as const, attempted_at: "2026-10-27T01:00:00Z", completed_at: null,
  failure: null, reason: null, http_status: null, error_code: null, retry_after: null,
};
const cases: Partial<TelegramNewRequest>[] = [
  { status: "waiting_telegram" },
  {},
  { prompt_delivery: { ...delivery, outcome: "sending" } },
  { prompt_delivery: { ...delivery, outcome: "rejected", reason: "rate_limited" } },
  { prompt_delivery: delivery },
  { prompt_delivery: { ...delivery, outcome: "unknown", reason: "forbidden" } },
  { prompt_delivery: { ...delivery, phase: "instructions", reason: "unauthorized" } },
];
const expected = {
  en: ["Open Telegram and press Start", "Delivery details were not recorded", "Sending the creation instructions", "Telegram rejected the creation message", "Telegram accepted the Create bot message", "Delivery of the Create bot message is unconfirmed", "Delivery of the Create bot message is unconfirmed"],
  "zh-CN": ["打开 Telegram 并点击 Start", "此设置未记录送达详情", "正在向 Telegram 发送创建说明和按钮", "Telegram 拒绝了创建消息", "Telegram 已接受 Create bot 消息", "尚未确认 Create bot 消息是否送达", "尚未确认 Create bot 消息是否送达"],
  "zh-TW": ["開啟 Telegram 並點選 Start", "此設定未記錄傳送詳情", "正在向 Telegram 傳送建立說明與按鈕", "Telegram 拒絕了建立訊息", "Telegram 已接受 Create bot 訊息", "尚未確認 Create bot 訊息是否送達", "尚未確認 Create bot 訊息是否送達"],
};
const reasonText = {
  en: ["Telegram is limiting requests", "An administrator may need to check the manager configuration"],
  "zh-CN": ["Telegram 正在限制请求", "可能需要管理员检查管理机器人的配置"],
  "zh-TW": ["Telegram 正在限制請求", "可能需要管理員檢查管理機器人的設定"],
};

afterEach(async () => { await act(() => appI18n.changeLanguage("en")); });

it.each(["en", "zh-CN", "zh-TW"] as const)("localizes dynamic delivery outcomes, safe reasons and recovery in %s", async (language) => {
  await act(() => appI18n.changeLanguage(language));
  for (const [index, overrides] of cases.entries()) {
    const view = render(<TelegramPromptStatus request={{ ...request, ...overrides }} recoveryUnexpired pending={false} recovering={false} regenerating={false} onRecover={vi.fn()} onRegenerate={vi.fn()} />);
    expect(screen.getByRole("status")).toHaveTextContent(expected[language][index]!);
    if (index === 3) expect(screen.getByRole("status")).toHaveTextContent(reasonText[language][0]!);
    if (index >= 5) expect(screen.getByRole("status")).toHaveTextContent(reasonText[language][1]!);
    if (index === 4) {
      const guidance = language === "en" ? /Already submitted Telegram’s form/ : language === "zh-CN" ? /已经提交 Telegram 表单/ : /已經提交 Telegram 表單/;
      const alternate = language === "en" ? /official mobile app/ : language === "zh-CN" ? /官方手机应用/ : /官方手機應用程式/;
      expect(screen.getByText(guidance)).toBeVisible();
      expect(screen.getByText(alternate)).toBeVisible();
    }
    view.unmount();
  }
});

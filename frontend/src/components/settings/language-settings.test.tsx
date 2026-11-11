vi.mock(
  "@/components/assistant/composer-editor",
  () => import("@/components/assistant/composer-editor.test-support"),
);
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { LanguageSettings } from "./language-settings";
import { DisplaySettings } from "./display-settings";
import { ChatComposer } from "@/components/assistant/chat-composer";
import { appI18n } from "@/i18n";
import { initialLanguage, LANGUAGE_STORAGE_KEY } from "@/i18n/language";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";

beforeEach(async () => {
  localStorage.clear();
  useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
  await appI18n.changeLanguage("en");
});
afterEach(async () => {
  cleanup();
  await act(async () => {
    await appI18n.changeLanguage("en");
  });
});

it("switches Studio settings and NyxBot together and preserves the chat draft", async () => {
  const onSend = vi.fn().mockResolvedValue(undefined);
  render(
    <>
      <LanguageSettings />
      <DisplaySettings />
      <ChatComposer
        ownerUserId="user-1"
        draftKey="conv:locale-test"
        active={false}
        sending={false}
        onSend={onSend}
        onStop={vi.fn()}
      />
    </>,
  );
  const editor = screen.getByPlaceholderText("Message NyxID Assistant...");
  fireEvent.change(editor, { target: { value: "Keep this draft 保留草稿" } });
  expect(
    screen.getByRole("button", { name: "Send message" }),
  ).toBeInTheDocument();

  fireEvent.change(screen.getByRole("combobox", { name: "Language" }), {
    target: { value: "zh-CN" },
  });
  expect(
    await screen.findByRole("radiogroup", { name: "主题" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "发送消息" })).toBeInTheDocument();
  expect(screen.getByPlaceholderText("给 NyxID 助手发送消息...")).toBe(editor);
  expect(editor).toHaveValue("Keep this draft 保留草稿");
  expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe("zh-CN");

  fireEvent.change(screen.getByRole("combobox", { name: "语言" }), {
    target: { value: "zh-TW" },
  });
  expect(
    await screen.findByRole("radiogroup", { name: "佈景主題" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "傳送訊息" })).toBeInTheDocument();
  expect(editor).toHaveValue("Keep this draft 保留草稿");
  expect(initialLanguage()).toBe("zh-TW");
  expect(document.documentElement.lang).toBe("zh-TW");
  expect(onSend).not.toHaveBeenCalled();
});

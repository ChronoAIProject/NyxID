import { fireEvent, render, screen, waitFor, cleanup } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { I18nextProvider } from "react-i18next";
import { createInstance } from "i18next";
import en from "../i18n/locales/en.json";
import zhCN from "../i18n/locales/zh-CN.json";
import zhTW from "../i18n/locales/zh-TW.json";
import { AGENT_INSTALL_PROMPT, InstallSkills, TERMINAL_INSTALL_COMMAND } from "./install-skills";

afterEach(cleanup);

describe("installation choices", () => {
  for (const [language, messages] of [["en", en], ["zh-CN", zhCN], ["zh-TW", zhTW]] as const) {
    it(`copies each selected payload exactly in ${language}`, async () => {
      const i18n = createInstance();
      await i18n.init({ lng: language, resources: { [language]: { translation: messages } } });
      const writeText = vi.fn().mockResolvedValue(undefined);
      Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
      render(<I18nextProvider i18n={i18n}><InstallSkills /></I18nextProvider>);
      fireEvent.click(screen.getByRole("button", { name: messages["install.copy"] }));
      await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(messages["install.codexPrompt"]));
      fireEvent.mouseDown(screen.getByRole("tab", { name: messages["install.terminal"] }), { button: 0, ctrlKey: false });
      expect(screen.getByRole("textbox")).toHaveValue(TERMINAL_INSTALL_COMMAND);
      fireEvent.click(screen.getByRole("button", { name: messages["install.copy"] }));
      await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(TERMINAL_INSTALL_COMMAND));
      fireEvent.mouseDown(screen.getByRole("tab", { name: messages["install.llm"] }), { button: 0, ctrlKey: false });
      fireEvent.change(screen.getByRole("combobox"), { target: { value: "other" } });
      fireEvent.click(screen.getByRole("button", { name: messages["install.copy"] }));
      await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(AGENT_INSTALL_PROMPT));
    });
  }
});

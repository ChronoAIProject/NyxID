import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContactSection } from "./contact";

const state = vi.hoisted(() => ({
  language: "en",
  searchStr: "?topic=bundle",
}));
vi.mock("@tanstack/react-router", () => ({ useLocation: () => state }));
vi.mock("./website-context", () => ({
  useWebsite: () => ({
    language: state.language,
    authHref: () => "/login?code=qa",
    siteHref: (path: string) => path,
  }),
}));

afterEach(() => {
  cleanup();
  state.language = "en";
  state.searchStr = "?topic=bundle";
});

describe("Contact interface pending delivery integration", () => {
  it.each([
    ["en", "Name", "Work email", "Company", "Topic", "Message", "Send message"],
    ["zh", "姓名", "工作邮箱", "公司", "主题", "留言", "发送"],
  ])(
    "restores five labelled %s fields and validates without pretending to send",
    async (language, name, email, company, topic, message, submit) => {
      state.language = language!;
      render(
        <ContactSection>
          <a href="https://discord.gg/QMvcs8UQBW">Discord</a>
        </ContactSection>,
      );
      expect(screen.getByLabelText(topic!)).toHaveValue("bundle");
      expect(screen.getByLabelText(new RegExp(company!))).not.toBeRequired();
      fireEvent.click(
        screen.getByRole("button", { name: submit, exact: true }),
      );
      expect(await screen.findAllByRole("alert")).toHaveLength(3);
      fireEvent.change(screen.getByLabelText(name!), {
        target: { value: "Contact QA" },
      });
      fireEvent.change(screen.getByLabelText(email!), {
        target: { value: "qa@example.com" },
      });
      fireEvent.change(screen.getByLabelText(message!), {
        target: { value: "A bundle question" },
      });
      fireEvent.click(
        screen.getByRole("button", { name: submit, exact: true }),
      );
      expect(await screen.findByRole("status")).toHaveTextContent(
        language === "zh" ? "尚未发送" : "Not sent",
      );
      expect(screen.getByLabelText(message!)).toHaveValue("A bundle question");
      expect(
        screen.queryByText(/Thanks, message sent|已发送，谢谢/),
      ).not.toBeInTheDocument();
    },
  );

  it("keeps a draft while the scripted bundle answer selects the form topic", () => {
    state.searchStr = "?topic=unsupported";
    render(
      <ContactSection>
        <a href="https://discord.gg/QMvcs8UQBW">Discord</a>
      </ContactSection>,
    );
    expect(screen.getByLabelText("Topic")).toHaveValue("");
    fireEvent.change(screen.getByLabelText("Message"), {
      target: { value: "Keep this draft" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: "I need a custom bundle" }),
    );
    expect(screen.getByLabelText("Topic")).toHaveValue("bundle");
    expect(screen.getByLabelText("Message")).toHaveValue("Keep this draft");
    expect(screen.getByRole("log")).toHaveTextContent(
      "Custom bundle is selected",
    );
    expect(screen.getByText(/Scripted FAQ demo/)).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Sign in to use NyxBot" }),
    ).toHaveAttribute("href", "/login?code=qa");
  });
});

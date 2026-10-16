import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContactSection } from "./contact";
import { openContactMailto } from "./contact-mailto";
import { websiteConfig } from "./config";

vi.mock("./contact-mailto", async (original) => ({
  ...(await original<typeof import("./contact-mailto")>()),
  openContactMailto: vi.fn(),
}));

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
  vi.mocked(openContactMailto).mockReset();
  vi.restoreAllMocks();
});

describe("Contact email-client handoff", () => {
  it.each([
    [
      "en",
      "Name",
      "Work email",
      "Company",
      "Topic",
      "Message",
      "Open email app",
    ],
    ["zh", "姓名", "工作邮箱", "公司", "主题", "留言", "打开邮件客户端"],
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
      expect(openContactMailto).not.toHaveBeenCalled();
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
        language === "zh" ? "尚未发送" : "has not been sent",
      );
      expect(openContactMailto).toHaveBeenCalledOnce();
      const draft = new URL(vi.mocked(openContactMailto).mock.calls[0]![0]);
      expect(draft.protocol).toBe("mailto:");
      expect(draft.pathname).toBe(websiteConfig.contactEmail);
      expect(draft.searchParams.get("body")).toContain("qa@example.com");
      expect(draft.searchParams.get("body")).toContain("A bundle question");
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

  it("keeps the draft and direct email link when client launch fails", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    vi.mocked(openContactMailto).mockImplementation(() => {
      throw new Error("No mail client");
    });
    render(
      <ContactSection>
        <span>Discord</span>
      </ContactSection>,
    );
    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "Contact QA" },
    });
    fireEvent.change(screen.getByLabelText("Work email"), {
      target: { value: "qa@example.com" },
    });
    fireEvent.change(screen.getByLabelText("Message"), {
      target: { value: "Keep this draft" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Open email app" }));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Could not open your email app",
    );
    expect(screen.getByLabelText("Message")).toHaveValue("Keep this draft");
    expect(
      screen.getByRole("link", { name: websiteConfig.contactEmail }),
    ).toHaveAttribute("href", `mailto:${websiteConfig.contactEmail}`);
    expect(
      screen.queryByText(/Thanks, message sent|已发送，谢谢/),
    ).not.toBeInTheDocument();
  });
});

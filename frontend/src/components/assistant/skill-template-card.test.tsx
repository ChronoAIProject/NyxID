import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SkillTemplateCard, type SkillTemplateOutline } from "./skill-template-card";

const outline: SkillTemplateOutline = {
  overview: "I send one text message.\nThen I check delivery.\n\nIt is not for marketing blasts.",
  steps: [
    { id: "step-1", title: "Check the number", text: "International format only.", asks_you: true },
    { id: "step-2", title: "Send it", text: "From your Twilio number.", uses: { name: "sms-send-twilio", version: "1.0" } },
  ],
  needs: "a Twilio connection.",
  produces: "one SMS.",
};

describe("SkillTemplateCard", () => {
  it("renders the outline as written, with step tags and Needs and produces", () => {
    render(<SkillTemplateCard name="text-a-customer" version="1.2" kind="skill" outline={outline} onStart={vi.fn()} onDismiss={vi.fn()} />);
    expect(screen.getByText("I send one text message. Then I check delivery.")).toBeInTheDocument();
    expect(screen.getByText("It is not for marketing blasts.")).toBeInTheDocument();
    const steps = screen.getByRole("list", { name: "Steps" });
    expect(steps).toHaveTextContent("Check the number. International format only.");
    expect(steps).toHaveTextContent("Asks you");
    expect(steps).toHaveTextContent("Uses sms-send-twilio1.0");
    expect(screen.getByText("a Twilio connection.")).toBeInTheDocument();
    expect(screen.getByText("one SMS.")).toBeInTheDocument();
  });

  it("starts or dismisses from the card, and attaches first when needed", () => {
    const onStart = vi.fn();
    const onDismiss = vi.fn();
    render(<SkillTemplateCard name="front-desk" version="1.0" kind="skillset" members={["a", "b"]} outline={outline} needsAttach onStart={onStart} onDismiss={onDismiss} />);
    expect(screen.getByText("Skillset · 2 skills")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add & start" }));
    fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(onStart).toHaveBeenCalledOnce();
    expect(onDismiss).toHaveBeenCalledOnce();
  });

  it("cannot start a skill without an outline", () => {
    render(<SkillTemplateCard name="legacy" version="1.0" kind="skill" outline={null} onStart={vi.fn()} />);
    expect(screen.getByText(/has no NYX template outline/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start" })).toBeDisabled();
  });

  it.each([
    [{ kind: "person", name: "Ada Example" }, "By Ada Example", null],
    [{ kind: "organization", name: "Example Org" }, "By Example Org", null],
    [{ kind: "platform", name: "Platform Skills" }, "By Platform Skills", null],
    [{ kind: "platform", name: "Platform Skills", service: "cma", system_skill: true }, "By Platform Skills", "System skill for cma"],
    [{ kind: "agent", name: "NyxBot", agent: { id: "nyxbot", name: "NyxBot", kind: "nyxbot" } }, "Drafted by NyxBot", null],
  ] as const)("shows who wrote the skill (%o)", (author, line, badge) => {
    render(<SkillTemplateCard name="s" version="1.0" kind="skill" outline={outline} author={author} />);
    expect(screen.getByText(line)).toBeInTheDocument();
    if (badge) expect(screen.getByText(badge)).toBeInTheDocument();
    else expect(screen.queryByText(/System skill/)).not.toBeInTheDocument();
  });

  it("shows no author line without an author", () => {
    render(<SkillTemplateCard name="s" version="1.0" kind="skill" outline={outline} />);
    expect(screen.queryByText(/^By |^Drafted by /)).not.toBeInTheDocument();
  });

  it("replaces the buttons once started", () => {
    render(<SkillTemplateCard name="text-a-customer" version="1.2" kind="skill" outline={outline} started onStart={vi.fn()} />);
    expect(screen.getByRole("status")).toHaveTextContent("Started. Progress continues below.");
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});

import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SkillRunChecklist, type SkillRunStep } from "./skill-run-checklist";

const steps: SkillRunStep[] = [
  { id: "step-1", title: "Get a number", status: "done", detail: "+65 6018 2245 reserved." },
  {
    id: "step-2",
    title: "Create the voice agent",
    status: "running",
    uses: { name: "voice-agent-telnyx", version: "1.0" },
    detail: "Writing its instructions.",
    substeps: [
      { id: "step-1", title: "Find or create the assistant", status: "done" },
      { id: "step-2", title: "Write its instructions", status: "running" },
    ],
  },
  { id: "step-3", title: "Choose the voice", status: "pending", substeps: [{ id: "step-1", title: "Hidden until started", status: "pending" }] },
];

describe("SkillRunChecklist", () => {
  it("marks the current step, its progress and the used skill's own steps", () => {
    const onStop = vi.fn();
    render(<SkillRunChecklist name="front-desk" version="1.0" kind="skillset" steps={steps} elapsed="0:42" onStop={onStop} />);
    expect(screen.getByText("Running skillset")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Step 2 of 3 · 0:42");
    expect(screen.getByRole("progressbar", { name: "Steps done" })).toHaveAttribute("aria-valuenow", "1");
    const list = screen.getByRole("list", { name: "Steps" });
    const current = within(list).getAllByRole("listitem").find((item) => item.getAttribute("aria-current") === "step");
    expect(current).toHaveTextContent("Create the voice agent");
    expect(within(current!).getAllByLabelText("Running")).toHaveLength(2);
    expect(within(current!).getByRole("list", { name: "voice-agent-telnyx steps" })).toHaveTextContent("Write its instructions");
    expect(screen.queryByText("Hidden until started")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(onStop).toHaveBeenCalledOnce();
  });

  it("asks for input on a waiting step", () => {
    render(
      <SkillRunChecklist
        name="front-desk"
        version="1.0"
        kind="skill"
        steps={[{ ...steps[0] }, { id: "step-2", title: "Choose the voice", status: "needs_you", detail: "Pick a voice." }]}
      />,
    );
    expect(screen.getByText("Running skill")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Step 2 of 2");
    expect(screen.getByText("Needs you")).toBeInTheDocument();
    expect(screen.getByText("Pick a voice.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();
  });

  it("finishes when every step is done or skipped", () => {
    render(
      <SkillRunChecklist
        name="front-desk"
        version="1.0"
        kind="skillset"
        steps={[{ ...steps[0] }, { id: "step-2", title: "Payment links", status: "skipped" }]}
        onStop={vi.fn()}
      />,
    );
    expect(screen.getByText("Done")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Finished · 2 steps");
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "2");
    expect(screen.queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();
  });
});

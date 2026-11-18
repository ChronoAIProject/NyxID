import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ReplyFeedback } from "@/hooks/use-assistant-feedback";
import { ReplyFeedbackFollowUp } from "./reply-feedback";

function form(comment: string, vote: "up" | "down" = "up") {
  const send = vi.fn().mockResolvedValue(true);
  const feedback: ReplyFeedback = {
    vote, comment, formOpen: true, pending: false, error: false,
    edit: vi.fn(), toggle: vi.fn(), close: vi.fn(), send,
  };
  render(<ReplyFeedbackFollowUp feedback={feedback} />);
  return send;
}

describe("saved feedback details", () => {
  it.each(["Useful, Accurate — clear example", "Useful, Accurate", "Accurate, Useful — clear example", "Just a note — with a dash"])("round-trips %s without changing it", async (comment) => {
    const send = form(comment);
    if (comment.startsWith("Useful")) {
      expect(screen.getByRole("button", { name: "Useful" })).toHaveAttribute("aria-pressed", "true");
      expect(screen.getByRole("button", { name: "Accurate" })).toHaveAttribute("aria-pressed", "true");
      expect(screen.getByRole("textbox")).toHaveValue(comment.includes(" — ") ? "clear example" : "");
    }
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Submit" })));
    expect(send).toHaveBeenCalledWith(comment);
  });

  it("does not duplicate saved tags when chips are toggled", async () => {
    const send = form("Useful — great");
    fireEvent.click(screen.getByRole("button", { name: "Useful" }));
    fireEvent.click(screen.getByRole("button", { name: "Useful" }));
    fireEvent.click(screen.getByRole("button", { name: "Accurate" }));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Submit" })));
    expect(send).toHaveBeenCalledWith("Useful, Accurate — great");
  });

  it("drops the previous vote's reasons and keeps its free text", async () => {
    const send = form("Useful, Accurate — keep this explanation", "down");
    expect(screen.getByRole("textbox")).toHaveValue("keep this explanation");
    expect(screen.getByRole("button", { name: "Not useful" })).toHaveAttribute("aria-pressed", "false");
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Submit" })));
    expect(send).toHaveBeenCalledWith("keep this explanation");
  });
});

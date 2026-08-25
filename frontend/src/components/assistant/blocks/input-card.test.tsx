import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { InputCardContentBlock } from "@/types/assistant";
import { InputCard } from "./input-card";

function inputCard(
  overrides: Partial<InputCardContentBlock> = {},
): InputCardContentBlock {
  return {
    type: "input_card",
    block_id: "input-card-1",
    request_id: "input-1",
    prompt: "Choose deployment regions",
    options: [
      { option_id: "option-sg", label: "Singapore" },
      { option_id: "option-fra", label: "Frankfurt" },
    ],
    allow_free_text: true,
    multi_select: true,
    state_version: 23,
    status: "pending",
    ...overrides,
  };
}

describe("InputCard", () => {
  it("submits opaque option ids without labels or free text", async () => {
    const onResolve = vi.fn().mockResolvedValue(undefined);
    render(<InputCard block={inputCard()} onResolve={onResolve} />);

    fireEvent.click(screen.getByLabelText("Singapore"));
    fireEvent.click(screen.getByLabelText("Frankfurt"));
    fireEvent.click(screen.getByRole("button", { name: "Submit" }));

    await waitFor(() =>
      expect(onResolve).toHaveBeenCalledWith({
        selectedOptionIds: ["option-sg", "option-fra"],
      }),
    );
  });

  it("submits trimmed free text as the other union branch", async () => {
    const onResolve = vi.fn().mockResolvedValue(undefined);
    render(
      <InputCard block={inputCard({ options: [] })} onResolve={onResolve} />,
    );

    fireEvent.change(screen.getByRole("textbox", { name: "Answer" }), {
      target: { value: "  Singapore north  " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Submit answer" }));

    await waitFor(() =>
      expect(onResolve).toHaveBeenCalledWith({ freeText: "Singapore north" }),
    );
  });

  it("disables every answer control behind the state-version fence", () => {
    render(
      <InputCard
        block={inputCard()}
        disabled
        onResolve={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    expect(screen.getByLabelText("Singapore")).toBeDisabled();
    expect(screen.getByLabelText("Frankfurt")).toBeDisabled();
    expect(screen.getByRole("textbox", { name: "Answer" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Submit" })).toBeDisabled();
  });
});

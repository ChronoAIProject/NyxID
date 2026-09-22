import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { NyxAgentModeSelector } from "./nyxagent-mode-selector";

afterEach(cleanup);

it("requires confirmation for full access and spells out the granted actions", async () => {
  const change = vi.fn().mockResolvedValue(undefined);
  render(<NyxAgentModeSelector mode="ask" disabled={false} onChange={change} />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("combobox", { name: "Mode" }));
  await user.click(screen.getByRole("option", { name: /Full access/ }));
  expect(change).not.toHaveBeenCalled();
  expect(screen.getByRole("dialog")).toHaveTextContent("all your connected services and nodes");
  expect(screen.getByRole("dialog")).toHaveTextContent("delete resources");
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(change).not.toHaveBeenCalled();
  await user.click(screen.getByRole("combobox", { name: "Mode" }));
  await user.click(screen.getByRole("option", { name: /Full access/ }));
  await user.click(screen.getByRole("button", { name: "Enable full access" }));
  await waitFor(() => expect(change).toHaveBeenCalledExactlyOnceWith("full"));
});

it("disables mode changes during a turn", () => {
  render(<NyxAgentModeSelector mode="full" disabled onChange={vi.fn()} />);
  expect(screen.getByRole("combobox", { name: "Mode" })).toBeDisabled();
});

it("returns to Ask without full-access confirmation and reports server refusal", async () => {
  const change = vi.fn().mockRejectedValue(new Error("A turn is already active"));
  render(<NyxAgentModeSelector mode="full" disabled={false} onChange={change} />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("combobox", { name: "Mode" }));
  await user.click(screen.getByRole("option", { name: /Ask before acting/ }));
  expect(await screen.findByRole("alert")).toHaveTextContent("A turn is already active");
  expect(change).toHaveBeenCalledWith("ask");
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  fireEvent.blur(screen.getByRole("combobox", { name: "Mode" }));
});

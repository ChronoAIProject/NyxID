import { beforeEach, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { TelegramManagers } from "./telegram-managers";
const fixture = vi.hoisted(() => ({
  save: vi.fn(),
  reset: vi.fn(),
  clear: vi.fn(),
}));
vi.mock("@/hooks/use-telegram-managers", () => ({
  useTelegramManagers: () => ({
    data: [
      {
        manager_bot_id: "100",
        username: "DefaultBot",
        ready: true,
        legacy_default: true,
      },
      {
        manager_bot_id: "200",
        username: "SecondBot",
        ready: true,
        legacy_default: false,
      },
    ],
  }),
  useConfigureTelegramManager: () => ({
    mutate: fixture.save,
    reset: fixture.reset,
  }),
  useClearTelegramManager: () => ({
    mutate: fixture.clear,
    reset: fixture.reset,
  }),
}));
beforeEach(() => {
  fixture.save.mockReset();
  fixture.reset.mockReset();
  fixture.clear.mockReset();
});
it("clears token input on submission and forgets mutation variables after settlement", async () => {
  render(<TelegramManagers />);
  const input = screen.getByLabelText("Additional manager bot token");
  expect(input).toHaveAttribute("type", "password");
  fireEvent.change(input, { target: { value: "200:fixture-token" } });
  fireEvent.click(screen.getByRole("button", { name: "Connect manager" }));
  await waitFor(() => expect(fixture.save).toHaveBeenCalledOnce());
  expect(input).toHaveValue("");
  expect(fixture.save.mock.calls[0]![0]).toBe("200:fixture-token");
  fixture.save.mock.calls[0]![1].onSettled();
  expect(fixture.reset).toHaveBeenCalled();
});
it("clears only the additional manager after explicit review", () => {
  render(<TelegramManagers />);
  expect(
    screen.queryByRole("button", { name: /clearing @DefaultBot/ }),
  ).not.toBeInTheDocument();
  fireEvent.click(
    screen.getByRole("button", { name: "Review clearing @SecondBot" }),
  );
  expect(fixture.clear).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Clear @SecondBot" }));
  expect(fixture.clear.mock.calls[0]![0]).toBe("200");
});

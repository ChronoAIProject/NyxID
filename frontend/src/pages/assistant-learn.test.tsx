import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { useIntroReplay } from "@/hooks/use-assistant-onboarding";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { AssistantLearnPage } from "./assistant-learn";

const { navigate, useFeature } = vi.hoisted(() => ({
  navigate: vi.fn(),
  useFeature: vi.fn<(flag: string) => boolean>(),
}));

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...await importOriginal<typeof import("@tanstack/react-router")>(),
  useNavigate: () => navigate,
  Link: ({ to, ...rest }: ComponentProps<"a"> & { to: string }) => <a href={to} {...rest} />,
}));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature }));

beforeEach(() => {
  navigate.mockClear();
  useFeature.mockImplementation((flag) => flag === FEATURE_FLAG.NYXAGENT_ENGINE);
  useIntroReplay.getState().consume();
});

it("shows all seven tasks by category, with their required inputs and results, when skill runs are off", () => {
  render(<AssistantLearnPage />);
  expect(screen.getByRole("heading", { level: 1, name: "Learn" })).toBeVisible();
  const browse = within(screen.getByRole("region", { name: "Browse by task" }));
  expect(browse.getAllByRole("button", { name: /^Try this/ })).toHaveLength(7);
  for (const category of ["Research", "Communications", "Operations", "Engineering"]) {
    const region = browse.getByRole("region", { name: category });
    expect(browse.getByRole("link", { name: category })).toHaveAttribute("href", `#${region.getAttribute("aria-labelledby")}`);
  }
  const operations = within(browse.getByRole("region", { name: "Operations" }));
  expect(operations.getByRole("heading", { name: "Explain weekly budget changes" })).toBeVisible();
  expect(operations.getByText("Weekly budget, actual spend, and reporting threshold")).toBeVisible();
  expect(operations.getByText(/highlight material differences/)).toBeVisible();
  const engineering = within(browse.getByRole("region", { name: "Engineering" }));
  expect(engineering.getByRole("heading", { name: "Assess release readiness" })).toBeVisible();
  expect(engineering.getByText(/Release scope, test results/)).toBeVisible();
  expect(engineering.getByText(/for a go\/no-go review/)).toBeVisible();
});

it("previews from the keyboard and returns focus to the exact trigger on Escape", async () => {
  const user = userEvent.setup();
  render(<AssistantLearnPage />);
  const operations = within(screen.getByRole("region", { name: "Operations" }));
  const trigger = operations.getByRole("button", { name: "Try this · Explain weekly budget changes" });
  trigger.focus();
  await user.keyboard("{Enter}");
  expect(screen.getByRole("dialog", { name: "Explain weekly budget changes" })).toBeVisible();
  await user.keyboard("{Escape}");
  await waitFor(() => expect(trigger).toHaveFocus());
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(navigate).not.toHaveBeenCalled();
});

it("replays the existing intro on the assistant home and links to the public usage guides", async () => {
  const user = userEvent.setup();
  render(<AssistantLearnPage />);
  const section = within(screen.getByRole("region", { name: "Getting started" }));
  expect(section.getByRole("link", { name: /Usage guides/ })).toHaveAttribute("href", "/docs");
  await user.click(section.getByRole("button", { name: /Replay intro/ }));
  expect(useIntroReplay.getState().pending).toBe(true);
  expect(navigate).toHaveBeenCalledWith({ to: "/assistant", search: {} });
});

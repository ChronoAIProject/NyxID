import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { AssistantLearnPage } from "./assistant-learn";

const { navigate, useFeature, readSkillPreview } = vi.hoisted(() => ({
  readSkillPreview: vi.fn(),
  navigate: vi.fn(),
  useFeature: vi.fn<(flag: string) => boolean>(),
}));

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...await importOriginal<typeof import("@tanstack/react-router")>(),
  useNavigate: () => navigate,
  Link: ({ to, search, ...rest }: ComponentProps<"a"> & { to: string; search?: { tour?: boolean } }) => <a href={search?.tour ? to + "?tour=true" : to} {...rest} />,
}));
vi.mock("@/hooks/use-agent-skills", () => ({ useAttachTemplateSkill: () => vi.fn(), readSkillPreview }));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature }));

beforeEach(() => {
  navigate.mockClear();
  useAuthStore.setState({ user: null });
  useFeature.mockImplementation((flag) => flag === FEATURE_FLAG.NYXAGENT_ENGINE);
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
  expect(operations.getByText("Reporting period · Budget and actuals · Material variances")).toBeVisible();
  expect(operations.getByText(/Variance table/)).toBeVisible();
  const engineering = within(browse.getByRole("region", { name: "Engineering" }));
  expect(engineering.getByRole("heading", { name: "Assess release readiness" })).toBeVisible();
  expect(engineering.getByText(/Release scope · Readiness evidence/)).toBeVisible();
  expect(engineering.getByText(/Go\/no-go recommendation/)).toBeVisible();
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

it("links to the workspace tour and public setup guides", () => {
  render(<AssistantLearnPage />);
  const section = within(screen.getByRole("region", { name: "Getting started" }));
  expect(section.getByRole("link", { name: /Usage guides/ })).toHaveAttribute("href", "/docs");
  expect(section.getByRole("link", { name: /Workspace tour/ })).toHaveAttribute("href", "/assistant?tour=true");
});

it("invalidates a pending catalog read on account switch and closes the previous owner’s dialog", async () => {
  const user = userEvent.setup();
  useAuthStore.setState({ user: { id: "owner-one" } as User });
  let guard: (() => void) | undefined;
  let rejectRead: ((error: Error) => void) | undefined;
  readSkillPreview.mockImplementationOnce((_id, _version, authorityGuard) => {
    guard = authorityGuard;
    return new Promise((_, reject) => { rejectRead = reject; });
  });
  render(<AssistantLearnPage />);
  const research = within(screen.getByRole("region", { name: "Research" }));
  await user.click(research.getByRole("button", { name: "Try this · Audit research claims" }));
  await user.click(screen.getByRole("button", { name: "Open NyxBot" }));
  expect(guard).toBeTypeOf("function");
  act(() => useAuthStore.setState({ user: { id: "owner-two" } as User }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(guard).toThrow("Template selection changed");
  await act(async () => { rejectRead?.(new Error("Stale request")); });
  expect(navigate).not.toHaveBeenCalled();
});

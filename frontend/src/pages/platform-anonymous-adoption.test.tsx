import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { AnonymousDraftReview } from "./platform-onboarding";
import {
  blankFacts,
  type OwnedBinding,
  type PlatformSetup,
} from "@/schemas/platform-onboarding";
const state = vi.hoisted(() => ({ drafts: [] as unknown[], mutate: vi.fn() }));
vi.mock("@/hooks/use-platform-onboarding", () => ({
  useAnonymousDrafts: () => ({ data: state.drafts }),
  useAdoptAnonymousDraft: () => ({ mutate: state.mutate, isPending: false }),
}));
const binding = { id: "binding", generation: 2 } as OwnedBinding;
const draft = {
  id: "draft",
  revision: 3,
  facts: { ...blankFacts(), name: "My Telegram cafe" },
} as PlatformSetup;
beforeEach(() => {
  state.drafts = [{ ...draft }];
  state.mutate.mockClear();
});
it("requires explicit review and submits the captured draft revision", () => {
  render(<AnonymousDraftReview binding={binding} />);
  expect(
    screen.getByRole("button", { name: "Adopt reviewed draft" }),
  ).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(screen.getByRole("button", { name: "Adopt reviewed draft" }));
  expect(state.mutate.mock.calls[0]![0]).toEqual(draft);
});
it("requires rereview when the draft revision changes", () => {
  const page = render(<AnonymousDraftReview binding={binding} />);
  fireEvent.click(screen.getByRole("checkbox"));
  state.drafts = [{ ...draft, revision: 4 }];
  page.rerender(<AnonymousDraftReview binding={binding} />);
  expect(
    screen.getByRole("button", { name: "Adopt reviewed draft" }),
  ).toBeDisabled();
  expect(state.mutate).not.toHaveBeenCalled();
});

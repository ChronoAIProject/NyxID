import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-error";
import { AnonymousDraftReview } from "./platform-onboarding";
import {
  blankFacts,
  type OwnedBinding,
  type PlatformSetup,
} from "@/schemas/platform-onboarding";
const state = vi.hoisted(() => ({ drafts: [] as unknown[], mutate: vi.fn() }));
vi.mock("@/hooks/use-platform-onboarding", () => ({
  useAnonymousDrafts: () => ({ data: { status: "ready", drafts: state.drafts } }),
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
it("submits rapid clicks once and allows a retry after the request settles", () => {
  render(<AnonymousDraftReview binding={binding} />);
  fireEvent.click(screen.getByRole("checkbox"));
  const adopt = screen.getByRole("button", { name: "Adopt reviewed draft" });
  fireEvent.click(adopt);
  fireEvent.click(adopt);
  expect(state.mutate).toHaveBeenCalledTimes(1);
  state.mutate.mock.calls[0]![1].onSettled();
  fireEvent.click(adopt);
  expect(state.mutate).toHaveBeenCalledTimes(2);
  expect(state.mutate.mock.calls[1]![0]).toEqual(draft);
});
it("clears consent on success and releases the guard for a newly reviewed draft", () => {
  render(<AnonymousDraftReview binding={binding} />);
  fireEvent.click(screen.getByRole("checkbox"));
  const adopt = screen.getByRole("button", { name: "Adopt reviewed draft" });
  fireEvent.click(adopt);
  act(() => {
    state.mutate.mock.calls[0]![1].onSuccess();
    state.mutate.mock.calls[0]![1].onSettled();
  });
  expect(screen.getByRole("checkbox")).not.toBeChecked();
  expect(adopt).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(adopt);
  expect(state.mutate).toHaveBeenCalledTimes(2);
});

it.each(["success", "refusal"] as const)(
  "keeps consent for a newer revision when an older submission settles: %s",
  (outcome) => {
    const page = render(<AnonymousDraftReview binding={binding} />);
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(
      screen.getByRole("button", { name: "Adopt reviewed draft" }),
    );
    const callbacks = state.mutate.mock.calls[0]![1];
    state.drafts = [{ ...draft, revision: 4 }];
    page.rerender(<AnonymousDraftReview binding={binding} />);
    fireEvent.click(screen.getByRole("checkbox"));
    act(() => {
      if (outcome === "success") callbacks.onSuccess();
      else
        callbacks.onError(
          new ApiError(409, {
            error: "conflict",
            error_code: -1,
            message: "Anonymous draft expired or changed",
          }),
        );
      callbacks.onSettled();
    });
    expect(screen.getByRole("checkbox")).toBeChecked();
    expect(
      screen.getByRole("button", { name: "Adopt reviewed draft" }),
    ).toBeEnabled();
    expect(state.mutate).toHaveBeenCalledTimes(1);
  },
);

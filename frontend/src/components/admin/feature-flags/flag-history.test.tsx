import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { FlagHistory } from "./flag-history";
const { mockHistory } = vi.hoisted(() => ({ mockHistory: vi.fn() }));
vi.mock("@/hooks/use-admin-feature-flags", () => ({
  useFeatureFlagHistory: mockHistory,
}));
const entry = {
  id: "event-1",
  actor_id: "user-1",
  actor_name: "Alex Chen",
  actor_email: "alex@example.test",
  event_type: "admin_feature_flag_override_set",
  details: { target_kind: "global", before: false, after: true, enabled: true },
  created_at: "2026-10-29T01:00:00Z",
};
beforeEach(() =>
  mockHistory.mockReset().mockReturnValue({
    data: {
      entries: [entry],
      next_cursor: null,
      legacy_ambiguous_omitted: false,
      origin: {
        created_by: "user-2",
        creator_name: "Sam Rivera",
        created_at: "2026-10-28T00:00:00Z",
      },
    },
    isLoading: false,
    error: null,
    isFetching: false,
  }),
);
it("shows durable creation attribution and exact toggle scope and values", () => {
  render(<FlagHistory flagKey="assistant:voice" enabled />);
  expect(screen.getByText("Sam Rivera")).toBeInTheDocument();
  expect(screen.getByText("Alex Chen")).toBeInTheDocument();
  expect(
    screen.getByText("Global · all users · Disabled → Enabled"),
  ).toBeInTheDocument();
  expect(screen.getByText("changed rollout")).toBeInTheDocument();
});
it("uses keyset cursors for older and newer history", () => {
  mockHistory.mockReturnValue({
    data: {
      entries: [entry],
      next_cursor: "123:cursor",
      origin: { created_by: null, creator_name: null, created_at: null },
    },
    isLoading: false,
    error: null,
    isFetching: false,
  });
  render(<FlagHistory flagKey="assistant:voice" enabled />);
  fireEvent.click(screen.getByRole("button", { name: "Older" }));
  expect(mockHistory).toHaveBeenLastCalledWith(
    "assistant:voice",
    "123:cursor",
    true,
  );
  fireEvent.click(screen.getByRole("button", { name: "Newer" }));
  expect(mockHistory).toHaveBeenLastCalledWith("assistant:voice", null, true);
});
it("marks legacy identity inference and explains code-defined origins", () => {
  mockHistory.mockReturnValue({
    data: {
      entries: [{ ...entry, legacy: true }],
      next_cursor: null,
      legacy_ambiguous_omitted: true,
      origin: { created_by: null, creator_name: null, created_at: null },
    },
    isLoading: false,
    error: null,
  });
  render(<FlagHistory flagKey="assistant:voice" enabled={false} />);
  expect(mockHistory).toHaveBeenCalledWith("assistant:voice", null, false);
  expect(screen.getByText(/Declared in code/)).toBeInTheDocument();
  expect(screen.getByText(/Legacy record/)).toBeInTheDocument();
  expect(
    screen.getByText(/Some older changes may have been omitted/),
  ).toBeInTheDocument();
});
it("shows sunset and tag changes with meaningful labels", () => {
  mockHistory.mockReturnValue({
    data: {
      entries: [
        {
          ...entry,
          event_type: "admin_feature_flag_metadata_set",
          details: {
            before: { sunset: false, tags: [] },
            after: { sunset: true, tags: ["voice"] },
          },
        },
      ],
      next_cursor: null,
      origin: { created_by: null, creator_name: null, created_at: null },
    },
    isLoading: false,
    error: null,
  });
  render(<FlagHistory flagKey="assistant:voice" enabled />);
  expect(screen.getByText("Status: Active → Sunset")).toBeInTheDocument();
  expect(screen.getByText("Tags: No tags → voice")).toBeInTheDocument();
});

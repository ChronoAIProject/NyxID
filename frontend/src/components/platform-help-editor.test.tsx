import { beforeEach, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import {
  PlatformHelpEditor,
  PlatformHelpAssignment,
} from "./platform-help-editor";
import type { PlatformProfile } from "@/schemas/platform-onboarding";
const fixture = vi.hoisted(() => ({
  mutate: vi.fn(),
  assign: vi.fn(),
  libraries: [
    {
      id: "49c91235-9f03-4d3f-b962-9748fd78f3c0",
      name: "Shared help",
      version: 1,
      published_revision: 1,
      updated_at: "2026-10-21T00:00:00Z",
      draft: {
        articles: [
          {
            id: "intro",
            title: "Welcome",
            body: "Published facts",
            tags: ["help"],
          },
        ],
        use_cases: [],
      },
    },
  ],
}));
vi.mock("@/hooks/use-platform-help", () => ({
  useHelpLibraries: () => ({ data: fixture.libraries }),
  useHelpMutation: () => ({ mutate: fixture.mutate, isPending: false }),
  useHelpHistory: () => ({
    data: [
      {
        revision: 1,
        content: fixture.libraries[0]!.draft,
        published_at: "2026-10-21T00:00:00Z",
      },
    ],
  }),
  useHelpAssignment: () => ({
    data: { library_id: fixture.libraries[0]!.id, version: 2 },
  }),
  useAssignHelp: () => ({ mutate: fixture.assign }),
}));
beforeEach(() => {
  fixture.mutate.mockReset();
  fixture.assign.mockReset();
  fixture.libraries[0] = {
    ...fixture.libraries[0]!,
    version: 1,
    draft: {
      articles: [
        {
          id: "intro",
          title: "Welcome",
          body: "Published facts",
          tags: ["help"],
        },
      ],
      use_cases: [],
    },
  };
});
it("edits a versioned draft and requires saving before publication", async () => {
  render(<PlatformHelpEditor />);
  fireEvent.change(screen.getByLabelText("Article text"), {
    target: { value: "Reviewed new facts" },
  });
  expect(
    screen.getByRole("button", { name: "Review publication" }),
  ).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Save draft" }));
  await waitFor(() => expect(fixture.mutate).toHaveBeenCalledOnce());
  expect(fixture.mutate.mock.calls[0]![0]).toMatchObject({
    action: "save",
    version: 1,
    content: { articles: [{ body: "Reviewed new facts" }] },
  });
});
it("keeps local edits on remote version drift until explicit discard", () => {
  const page = render(<PlatformHelpEditor />);
  fireEvent.change(screen.getByLabelText("Article text"), {
    target: { value: "Unsaved fact" },
  });
  fixture.libraries[0] = {
    ...fixture.libraries[0]!,
    version: 2,
    draft: {
      articles: [
        { id: "intro", title: "Welcome", body: "Remote fact", tags: [] },
      ],
      use_cases: [],
    },
  };
  page.rerender(<PlatformHelpEditor />);
  expect(screen.getByLabelText("Article text")).toHaveValue("Unsaved fact");
  expect(screen.getByRole("button", { name: "Save draft" })).toBeDisabled();
  expect(screen.getByRole("alert")).toHaveTextContent("changed elsewhere");
  fireEvent.click(
    screen.getByRole("button", { name: "Discard local edits and reload" }),
  );
  expect(screen.getByLabelText("Article text")).toHaveValue("Remote fact");
});
it("previews text safely and publishes only after review", () => {
  fixture.libraries[0]!.draft.articles[0]!.body =
    "<script>untrusted content</script>";
  const page = render(<PlatformHelpEditor />);
  expect(
    screen.queryByRole("button", { name: "Publish shared help" }),
  ).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Review publication" }));
  expect(screen.getByLabelText("Help preview")).toHaveTextContent(
    "<script>untrusted content</script>",
  );
  expect(page.container.querySelector("script")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Publish shared help" }));
  expect(fixture.mutate.mock.calls[0]![0]).toEqual({
    action: "publish",
    id: fixture.libraries[0]!.id,
    version: 1,
  });
});
it("requires a reviewed revision before restoring history", () => {
  render(<PlatformHelpEditor />);
  fireEvent.click(screen.getByText("Published history and rollback"));
  fireEvent.click(screen.getByRole("button", { name: "Review revision 1" }));
  expect(fixture.mutate).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Restore revision 1" }));
  expect(fixture.mutate.mock.calls[0]![0]).toMatchObject({
    action: "publish",
    version: 1,
    rollback_revision: 1,
  });
});
it("does not write an unchanged manager assignment", () => {
  render(
    <PlatformHelpAssignment
      profile={{ id: "manager", generation: 9 } as PlatformProfile}
    />,
  );
  expect(
    screen.getByRole("button", { name: "Use this library" }),
  ).toBeDisabled();
  expect(fixture.assign).not.toHaveBeenCalled();
});

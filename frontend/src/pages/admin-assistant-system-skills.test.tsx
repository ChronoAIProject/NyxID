import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { AdminAssistantSystemSkillsPage } from "./admin-assistant-system-skills";
const mocks = vi.hoisted(() => ({
  current: vi.fn(),
  write: vi.fn(),
  mutate: vi.fn(),
  reload: vi.fn(),
}));
vi.mock("@/hooks/use-assistant-system-skills", () => ({
  useSystemSkills: mocks.current,
  useSetSystemSkills: mocks.write,
  useSystemSkillSearch: () => ({
    data: {
      items: [{ id: "new", name: "New guide", description: "Guide" }],
      total_pages: 1,
    },
  }),
  useSystemSkillVersions: () => ({
    data: { items: [{ version: "1.0", deprecated: false }], total_pages: 1 },
  }),
  useSystemSkillPreview: () => ({
    data: {
      reference: {
        source: "ornn",
        skill_id: "new",
        name: "New guide",
        version: "1.0",
        sha256: "a".repeat(64),
        dependencies: [],
      },
      description: "Public guide",
      size_bytes: 100,
    },
  }),
}));
const skill = (id: string) => ({
  skill_id: id,
  name: id,
  version: "1.0",
  sha256: "a".repeat(64),
  dependencies: [],
  description: `${id} guide`,
  size_bytes: 100,
});
beforeEach(() => {
  vi.clearAllMocks();
  mocks.current.mockReturnValue({
    data: {
      revision: 3,
      skills: [skill("first"), skill("second")],
      updated_by: "admin",
      updated_at: null,
    },
    refetch: mocks.reload,
  });
  mocks.write.mockReturnValue({
    mutate: mocks.mutate,
    isPending: false,
    error: null,
  });
});
it("shows metadata, dirty-gates save and sends reordered exact selections with the observed revision", async () => {
  render(<AdminAssistantSystemSkillsPage />);
  const save = screen.getByRole("button", { name: "Save built-in skills" });
  expect(save).toBeDisabled();
  expect(screen.getByText("first guide")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Move second up" }));
  await waitFor(() => expect(save).toBeEnabled());
  fireEvent.click(save);
  await waitFor(() =>
    expect(mocks.mutate).toHaveBeenCalledWith({
      expected_revision: 3,
      skills: [
        { skill: "second", version: "1.0" },
        { skill: "first", version: "1.0" },
      ],
    }),
  );
});
it("clears all selections without supplying client hashes", async () => {
  render(<AdminAssistantSystemSkillsPage />);
  fireEvent.click(screen.getByRole("button", { name: "Remove first" }));
  fireEvent.click(screen.getByRole("button", { name: "Remove second" }));
  const save = screen.getByRole("button", { name: "Save built-in skills" });
  await waitFor(() => expect(save).toBeEnabled());
  fireEvent.click(save);
  await waitFor(() =>
    expect(mocks.mutate).toHaveBeenCalledWith({
      expected_revision: 3,
      skills: [],
    }),
  );
});
it("offers a reload on revision conflict and blocks further saves", () => {
  mocks.write.mockReturnValue({
    mutate: mocks.mutate,
    isPending: false,
    error: Object.assign(new Error("System skills changed"), { status: 409 }),
  });
  render(<AdminAssistantSystemSkillsPage />);
  fireEvent.click(
    screen.getByRole("button", { name: "Reload current revision" }),
  );
  expect(mocks.reload).toHaveBeenCalled();
  expect(
    screen.getByRole("button", { name: "Save built-in skills" }),
  ).toBeDisabled();
});
it("adds a searched exact version without sending preview hashes", async () => {
  render(<AdminAssistantSystemSkillsPage />);
  fireEvent.click(screen.getByRole("button", { name: "New guide" }));
  fireEvent.click(screen.getByRole("button", { name: "Add skill" }));
  const save = screen.getByRole("button", { name: "Save built-in skills" });
  await waitFor(() => expect(save).toBeEnabled());
  fireEvent.click(save);
  await waitFor(() =>
    expect(mocks.mutate).toHaveBeenCalledWith({
      expected_revision: 3,
      skills: [
        { skill: "first", version: "1.0" },
        { skill: "second", version: "1.0" },
        { skill: "new", version: "1.0" },
      ],
    }),
  );
});

it("sends only one PUT while a save is pending, including direct form submission", async () => {
  const hooks = await vi.importActual<typeof import("@/hooks/use-assistant-system-skills")>("@/hooks/use-assistant-system-skills");
  mocks.write.mockImplementation(hooks.useSetSystemSkills);
  let resolveWrite!: (value: unknown) => void;
  const put = vi.spyOn(api, "put").mockImplementation(() => new Promise((resolve) => { resolveWrite = resolve; }));
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  try {
    render(<QueryClientProvider client={client}><AdminAssistantSystemSkillsPage /></QueryClientProvider>);
    fireEvent.click(screen.getByRole("button", { name: "Move second up" }));
    const save = screen.getByRole("button", { name: "Save built-in skills" });
    await waitFor(() => expect(save).toBeEnabled());
    fireEvent.click(save);
    await waitFor(() => expect(put).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(save).toBeDisabled());
    fireEvent.click(save);
    await act(async () => { fireEvent.submit(save.closest("form")!); });
    expect(put).toHaveBeenCalledTimes(1);
    await act(async () => { resolveWrite({ revision: 4, skills: [skill("second"), skill("first")], updated_by: "admin", updated_at: null }); });
  } finally {
    put.mockRestore();
    client.clear();
  }
});

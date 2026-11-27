import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentAvatarEditor } from "./agent-avatar-editor";
import {
  assistantAgentSchema,
  type AssistantAgent,
} from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";
import {
  defaultAvatar,
  type AvatarConfig,
} from "@/lib/assistant/agent-avatar-model";
const fixture = vi.hoisted(() => ({
  read: vi.fn(),
  write: vi.fn(),
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useUpdateAgentAvatar: () => ({
    mutateAsync: fixture.write,
    isPending: false,
  }),
}));
vi.mock("@/lib/assistant/nyxbot-api", () => ({
  nyxBotApi: { agent: fixture.read },
}));
vi.mock("@/components/ui/change-review", () => ({
  ChangeReview: ({
    children,
    save,
  }: {
    children: (review: () => void) => React.ReactNode;
    save: () => Promise<void>;
  }) =>
    children(() => {
      void save();
    }),
}));
vi.mock("@/components/ui/grouped-form", () => ({
  GroupedFormBar: () => null,
}));
vi.mock("@/lib/assistant/use-avatar-environment", () => ({
  useAnimationActive: () => false,
  useReducedAvatarMotion: () => true,
  useDocumentTheme: () => "dark",
}));
let server: AssistantAgent;
beforeEach(() => {
  useAuthStore.setState({
    user: {
      id: "u",
      capabilities: { enabled_features: ["assistant:avatars"] },
    } as never,
  });
  server = assistantAgentSchema.parse({
    id: "nyx",
    name: "NyxBot",
    kind: "nyxbot",
    status: "idle",
    created_at: "2026-11-25T00:00:00Z",
    last_active_at: "2026-11-25T00:00:00Z",
  });
  fixture.read.mockReset().mockImplementation(async () => ({ agent: server }));
  fixture.write
    .mockReset()
    .mockImplementation(async ({ avatar }: { avatar: AvatarConfig | null }) => {
      server = {
        ...server,
        avatar: avatar
          ? {
              template: avatar.template,
              choices: Object.fromEntries(
                Object.entries(avatar.choices).sort(),
              ),
            }
          : null,
      };
    });
});
afterEach(() => {
  cleanup();
  useAuthStore.setState({ user: null });
});
function renderEditor(agent: AssistantAgent) {
  const view = render(<AgentAvatarEditor agent={agent} />);
  fireEvent.click(screen.getByRole("button", { name: "Edit avatar" }));
  return view;
}
const choose = (name: string) =>
  fireEvent.click(screen.getByRole("radio", { name }));
it("saves twice despite server map sorting, and scopes a draft to its agent", async () => {
  const view = renderEditor(server);
  choose("Happy");
  fireEvent.click(screen.getByText("Save avatar"));
  await waitFor(() => expect(fixture.write).toHaveBeenCalledTimes(1));
  choose("Wink");
  fireEvent.click(screen.getByText("Save avatar"));
  await waitFor(() => expect(fixture.write).toHaveBeenCalledTimes(2));
  expect(server.avatar?.choices.eyes).toBe("wink");
  choose("Calm");
  const next = assistantAgentSchema.parse({
    id: "other",
    name: "NyxBot",
    kind: "nyxbot",
    status: "idle",
    created_at: "2026-11-25T00:00:00Z",
    last_active_at: "2026-11-25T00:00:00Z",
  });
  view.rerender(<AgentAvatarEditor agent={next} />);
  fireEvent.click(screen.getByRole("button", { name: "Edit avatar" }));
  expect(
    screen
      .getAllByRole("radio", { name: "Tall" })
      .find((el) => el.getAttribute("aria-checked") === "true"),
  ).toHaveAttribute("aria-checked", "true");
  expect(screen.getByText("Save avatar")).toBeDisabled();
});
it("locks the complete preflight and mutation, blocks late edits, and unlocks on error", async () => {
  let resolve!: (value: { agent: AssistantAgent }) => void;
  fixture.read.mockImplementationOnce(
    () =>
      new Promise((r) => {
        resolve = r;
      }),
  );
  renderEditor(server);
  choose("Happy");
  fireEvent.click(screen.getByText("Save avatar"));
  fireEvent.click(screen.getByText("Save avatar"));
  await waitFor(() => expect(fixture.read).toHaveBeenCalledTimes(1));
  expect(screen.getByText("Save avatar")).toBeDisabled();
  choose("Wink");
  expect(screen.getByRole("radio", { name: "Happy" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await act(async () => resolve({ agent: server }));
  await waitFor(() => expect(fixture.write).toHaveBeenCalledTimes(1));
  expect(server.avatar?.choices.eyes).toBe("happy");
  choose("Calm");
  fixture.read.mockRejectedValueOnce(new Error("preflight failed"));
  fireEvent.click(screen.getByText("Save avatar"));
  await screen.findByRole("alert");
  expect(screen.getByText("Save avatar")).toBeEnabled();
  expect(screen.getByRole("radio", { name: "Calm" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
});
it("requires explicit adoption of an unknown appearance and never saves preview states", async () => {
  server = {
    ...server,
    avatar: { template: "face-v9", choices: { future_choice: "future-value" } },
  };
  renderEditor(server);
  expect(screen.getByText("Save avatar")).toBeDisabled();
  expect(screen.queryByRole("radio", { name: "Wink" })).toBeNull();
  fireEvent.click(screen.getByText("Adopt face kit"));
  choose("Wink");
  fireEvent.click(screen.getByRole("button", { name: "Motion" }));
  fireEvent.change(
    screen.getByRole("combobox", { name: "Preview expression" }),
    { target: { value: "success" } },
  );
  fireEvent.click(screen.getByText("Save avatar"));
  await waitFor(() => expect(fixture.write).toHaveBeenCalledTimes(1));
  const saved = fixture.write.mock.calls[0]![0].avatar;
  expect(saved.template).toBe("face-v1");
  expect(saved.choices.eyes).toBe("wink");
  expect(saved.choices).not.toHaveProperty("preview");
  expect(saved.choices).not.toHaveProperty("state");
});
it("hides editing without authority or rollout", () => {
  const view = render(
    <AgentAvatarEditor agent={{ ...server, can_maintain: false }} />,
  );
  expect(view.container).toBeEmptyDOMElement();
  useAuthStore.setState({ user: null });
  view.rerender(<AgentAvatarEditor agent={server} />);
  expect(view.container).toBeEmptyDOMElement();
});

it("discards from the confirmed saved baseline before props refresh and has no download controls", async () => {
  renderEditor(server);
  expect(screen.queryByText("Download avatar")).not.toBeInTheDocument();
  expect(screen.queryByText(/Export saved avatar/)).not.toBeInTheDocument();
  choose("Happy");
  fireEvent.click(screen.getByText("Save avatar"));
  await waitFor(() => expect(screen.getByText("Save avatar")).toBeDisabled());
  expect(screen.getByRole("status")).toHaveTextContent("Avatar saved.");
  choose("Wink");
  fireEvent.click(screen.getByRole("button", { name: "Discard avatar" }));
  expect(screen.getByRole("radio", { name: "Happy" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
});
it("does not start an old save after unmount or acting-account change during preflight", async () => {
  let finish!: (value: { agent: AssistantAgent }) => void;
  fixture.read.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const view = renderEditor(server);
  choose("Happy");
  fireEvent.click(screen.getByText("Save avatar"));
  await act(async () =>
    useAuthStore.setState({
      user: {
        id: "other",
        capabilities: { enabled_features: ["assistant:avatars"] },
      } as never,
    }),
  );
  await act(async () => finish({ agent: server }));
  expect(fixture.write).not.toHaveBeenCalled();
  view.unmount();
});

it("navigates one step at a time without dirtying or saving preview controls", () => {
  renderEditor(server);
  expect(screen.getByRole("button", { name: "Face" })).toHaveAttribute(
    "aria-current",
    "step",
  );
  expect(
    screen.queryByRole("radiogroup", { name: "Colour" }),
  ).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /^Next:/ }));
  expect(screen.getByRole("button", { name: "Style" })).toHaveFocus();
  expect(
    screen.queryByRole("radiogroup", { name: "Head" }),
  ).not.toBeInTheDocument();
  fireEvent.change(screen.getByRole("combobox", { name: "Colour palette" }), {
    target: { value: "Deep" },
  });
  expect(screen.getByRole("radio", { name: "Midnight" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /^Next:/ }));
  fireEvent.change(
    screen.getByRole("combobox", { name: "Preview expression" }),
    { target: { value: "speaking" } },
  );
  expect(screen.getByText("Save avatar")).toBeDisabled();
  expect(fixture.read).not.toHaveBeenCalled();
  expect(fixture.write).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Back" }));
  expect(screen.getByRole("button", { name: "Style" })).toHaveAttribute(
    "aria-current",
    "step",
  );
});
it("keeps edits across steps and retains collapsed appearance and reaction choices in the save", async () => {
  const initial = defaultAvatar(server.id, server.kind);
  server = {
    ...server,
    avatar: {
      ...initial,
      choices: {
        ...initial.choices,
        treatment: "glass",
        background: "dark",
        frame: "circle",
        react_thinking: "character-churn",
      },
    },
  };
  renderEditor(server);
  choose("Wink");
  fireEvent.click(screen.getByRole("button", { name: /^Next:/ }));
  fireEvent.change(screen.getByRole("combobox", { name: "Colour palette" }), {
    target: { value: "Deep" },
  });
  choose("Plum");
  choose("Dither");
  fireEvent.change(screen.getByRole("combobox", { name: "Texture style" }), {
    target: { value: "dither-halftone" },
  });
  fireEvent.click(screen.getByRole("button", { name: /^Next:/ }));
  choose("Aurora");
  fireEvent.click(screen.getByRole("button", { name: "Face" }));
  expect(screen.getByRole("radio", { name: "Wink" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  fireEvent.click(screen.getByRole("button", { name: "Style" }));
  expect(screen.getByRole("radio", { name: "Plum" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  expect(screen.getByRole("combobox", { name: "Texture style" })).toHaveValue(
    "dither-halftone",
  );
  fireEvent.click(screen.getByText("Save avatar"));
  await waitFor(() => expect(fixture.write).toHaveBeenCalledTimes(1));
  expect(server.avatar?.choices).toMatchObject({
    eyes: "wink",
    palette: "deep-plum",
    surface: "dither-halftone",
    ripple_palette: "aurora",
    treatment: "glass",
    background: "dark",
    frame: "circle",
    react_thinking: "character-churn",
  });
});
it("keeps incompatible texture choices explained and preserves the saved texture when changing steps", () => {
  const initial = defaultAvatar(server.id, server.kind);
  server = {
    ...server,
    avatar: {
      ...initial,
      choices: { ...initial.choices, treatment: "neon-line" },
    },
  };
  renderEditor(server);
  fireEvent.click(screen.getByRole("button", { name: "Style" }));
  expect(screen.getByRole("radio", { name: "Dither" })).toBeDisabled();
  expect(
    screen.getByText(
      "Textured styles need a filled face. Choose another colour treatment before selecting a texture.",
    ),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByText("More appearance options"));
  fireEvent.change(screen.getByRole("combobox", { name: "Finish" }), {
    target: { value: "glass" },
  });
  choose("ASCII");
  const texture = screen.getByRole("combobox", { name: "Texture style" });
  fireEvent.change(texture, { target: { value: "ascii-braille" } });
  fireEvent.click(screen.getByRole("button", { name: "Motion" }));
  fireEvent.click(screen.getByRole("button", { name: "Style" }));
  expect(screen.getByRole("combobox", { name: "Texture style" })).toHaveValue(
    "ascii-braille",
  );
  expect(
    within(screen.getByRole("radiogroup", { name: "Texture" })).getByRole(
      "radio",
      { checked: true },
    ),
  ).toHaveAccessibleName("ASCII");
});

it("keeps the explicit save action on every step and retries a failed write without losing the draft", async () => {
  renderEditor(server);
  expect(screen.getByText("Step 1 of 3")).toBeInTheDocument();
  expect(
    screen.getByRole("heading", { name: "Choose a face" }),
  ).toBeInTheDocument();
  choose("Happy");
  for (const name of ["Next: Style", "Next: Motion"]) {
    fireEvent.click(screen.getByRole("button", { name }));
    expect(screen.getByRole("button", { name: "Save avatar" })).toBeEnabled();
  }
  fixture.write.mockRejectedValueOnce(new Error("Save failed"));
  fireEvent.click(screen.getByRole("button", { name: "Save avatar" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Save failed");
  expect(screen.getByRole("button", { name: "Save avatar" })).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "Save avatar" }));
  await waitFor(() =>
    expect(screen.getByRole("status")).toHaveTextContent("Avatar saved."),
  );
  expect(server.avatar?.choices.eyes).toBe("happy");
});

it("keeps Overview compact and opens choices on a separate drawer page", () => {
  render(<AgentAvatarEditor agent={server} />);
  const trigger = screen.getByRole("button", { name: "Edit avatar" });
  expect(screen.getByRole("img", { name: "Saved avatar" })).toBeInTheDocument();
  expect(
    screen.queryByRole("form", { name: "Avatar" }),
  ).not.toBeInTheDocument();
  fireEvent.click(trigger);
  expect(screen.getByRole("heading", { name: "Edit avatar" })).toHaveFocus();
  expect(screen.getByRole("radio", { name: "Nyx" })).toBeInTheDocument();
  expect(
    screen.queryByRole("radio", { name: "NyxID logo" }),
  ).not.toBeInTheDocument();
  expect(screen.queryByText("In the app")).not.toBeInTheDocument();
  expect(screen.queryByLabelText("Compact previews")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Back to Overview" }));
  expect(screen.getByRole("button", { name: "Edit avatar" })).toHaveFocus();
  expect(
    screen.queryByRole("form", { name: "Avatar" }),
  ).not.toBeInTheDocument();
});
it("shows the confirmed saved appearance in Overview before parent props refresh", async () => {
  renderEditor(server);
  choose("Happy");
  fireEvent.click(screen.getByRole("button", { name: "Save avatar" }));
  await waitFor(() =>
    expect(screen.getByRole("status")).toHaveTextContent("Avatar saved."),
  );
  fireEvent.click(screen.getByRole("button", { name: "Back to Overview" }));
  fireEvent.click(screen.getByRole("button", { name: "Edit avatar" }));
  expect(screen.getByRole("radio", { name: "Happy" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  expect(screen.getByRole("button", { name: "Save avatar" })).toBeDisabled();
});

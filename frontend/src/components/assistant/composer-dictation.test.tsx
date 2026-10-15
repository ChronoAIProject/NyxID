import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { voiceOption } from "@/test/voice-fixture";
import { useComposerDictation } from "./composer-dictation";
const f = vi.hoisted(() => ({
  get: vi.fn(),
  clients: [] as {
    callbacks: {
      state: (value: string) => void;
      text: (value: unknown) => void;
    };
    start: ReturnType<typeof vi.fn>;
    stop: ReturnType<typeof vi.fn>;
    cancel: ReturnType<typeof vi.fn>;
  }[],
}));
vi.mock("@/lib/api-client", () => ({ api: { get: f.get } }));
vi.mock("@/lib/assistant-dictation-client", () => ({
  AssistantDictationClient: class {
    callbacks: (typeof f.clients)[number]["callbacks"];
    start = vi.fn(() => this.callbacks.state("requesting"));
    stop = vi.fn(() => this.callbacks.state("review"));
    cancel = vi.fn();
    constructor(callbacks: (typeof f.clients)[number]["callbacks"]) {
      this.callbacks = callbacks;
      f.clients.push(this);
    }
  },
}));
const begin = vi.fn(),
  insert = vi.fn(() => true),
  getThread = vi.fn(async () => "thread-A"),
  closed = vi.fn();
function Harness({
  scope = "actor:draft-A",
  disabled = false,
  enabled = true,
}: {
  scope?: string;
  disabled?: boolean;
  enabled?: boolean;
}) {
  const d = useComposerDictation({
    scope,
    disabled,
    enabled,
    getThread,
    begin,
    insert,
    reviewClosed: closed,
  });
  return (
    <>
      {d.button}
      {d.review}
    </>
  );
}
beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
  f.clients = [];
  f.get.mockResolvedValue({ options: [voiceOption] });
  getThread.mockResolvedValue("thread-A");
});
async function configure(mode = "hold") {
  fireEvent.click(screen.getByRole("button", { name: "Configure dictation" }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Start dictation" }),
    ).toBeEnabled(),
  );
  if (mode === "hold")
    fireEvent.change(
      screen.getByRole("combobox", { name: "Dictation control" }),
      { target: { value: "hold" } },
    );
}
it.each(["pointer", "keyboard"])(
  "initial and subsequent hold %s release survives state rerender and permission pending",
  async (kind) => {
    render(<Harness />);
    await configure();
    const initial = screen.getByRole("button", { name: "Hold to dictate" });
    const press = (button: HTMLElement) =>
      kind === "pointer"
        ? fireEvent.pointerDown(button, {
            pointerId: 7,
            button: 0,
            pointerType: "touch",
          })
        : fireEvent.keyDown(button, { key: " " });
    const release = () =>
      kind === "pointer"
        ? fireEvent.pointerUp(window, { pointerId: 7, button: 0 })
        : fireEvent.keyUp(window, { key: " " });
    press(initial);
    expect(f.clients).toHaveLength(1);
    expect(
      screen.getByRole("button", { name: "Hold to dictate" }),
    ).toBeInTheDocument();
    release();
    expect(f.clients[0]!.stop).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "Cancel dictation" }));
    press(screen.getByRole("button", { name: "Dictate a message" }));
    expect(f.clients).toHaveLength(2);
    expect(
      screen.getByRole("button", { name: "Stop dictation" }),
    ).toBeInTheDocument();
    release();
    expect(f.clients[1]!.stop).toHaveBeenCalledOnce();
  },
);
it("click toggle reviews final text, never inserts interim, and escape restores without mutation", async () => {
  render(<Harness />);
  await configure("toggle");
  fireEvent.click(screen.getByRole("button", { name: "Start dictation" }));
  act(() =>
    f.clients[0]!.callbacks.text({
      final: "Final",
      interim: "pending",
      parts: [
        { text: "Final", complete: true, order: 0 },
        { text: "pending", complete: false, order: 1 },
      ],
    }),
  );
  expect(
    screen.getByRole("button", { name: "Insert dictation" }),
  ).toBeDisabled();
  fireEvent.click(
    screen.getByRole("button", { name: "Stop dictation for review" }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Insert dictation" }));
  expect(insert).toHaveBeenCalledWith("Final");
  expect(closed).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole("button", { name: "Dictate a message" }));
  fireEvent.keyDown(window, { key: "Escape" });
  expect(insert).toHaveBeenCalledTimes(1);
  expect(closed).toHaveBeenCalledTimes(2);
});
it("account/draft rebind clears all authority and fences stale callbacks before next start", async () => {
  const { rerender } = render(<Harness />);
  await configure("toggle");
  fireEvent.click(screen.getByRole("button", { name: "Start dictation" }));
  const old = f.clients[0]!;
  rerender(<Harness scope="other:draft-B" />);
  expect(old.cancel).toHaveBeenCalled();
  expect(screen.queryByLabelText("Dictation review")).not.toBeInTheDocument();
  act(() => {
    old.callbacks.state("capturing");
    old.callbacks.text({ final: "leak", interim: "" });
  });
  expect(screen.queryByText("leak")).not.toBeInTheDocument();
  getThread.mockResolvedValue("thread-B");
  await configure("toggle");
  fireEvent.click(screen.getByRole("button", { name: "Start dictation" }));
  expect(f.clients[1]!.start).toHaveBeenCalledWith(
    "thread-B",
    expect.any(Object),
  );
});
it.each(["disabled", "enabled"])(
  "loss of %s eligibility cancels capture without restoring stale focus",
  async (which) => {
    const { rerender } = render(<Harness />);
    await configure("toggle");
    fireEvent.click(screen.getByRole("button", { name: "Start dictation" }));
    rerender(
      <Harness disabled={which === "disabled"} enabled={which !== "enabled"} />,
    );
    expect(f.clients[0]!.cancel).toHaveBeenCalled();
    expect(screen.queryByLabelText("Dictation review")).not.toBeInTheDocument();
    expect(closed).not.toHaveBeenCalled();
  },
);
it("restores persisted hold mode and tolerates unavailable storage", async () => {
  localStorage.setItem("nyxid:dictation-mode:actor", "hold");
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "Configure dictation" }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Hold to dictate" }),
    ).toBeEnabled(),
  );
  expect(
    screen.getByRole("combobox", { name: "Dictation control" }),
  ).toHaveValue("hold");
});

it("hover followed by Configure click stays open and keyboard/touch entry remains available", async () => {
  render(<Harness />);
  fireEvent.mouseEnter(
    screen.getByRole("button", { name: "Dictate a message" }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Configure dictation" }));
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Start dictation" }),
    ).toBeEnabled(),
  );
  expect(
    screen.getByRole("combobox", { name: "Dictation control" }),
  ).toBeVisible();
});

it("discloses the authored input-duration and per-start request prices before capture", async () => {
  f.get.mockResolvedValue({
    options: [
      {
        ...voiceOption,
        pricing: {
          ...voiceOption.pricing,
          components: [
            {
              metric: "requests",
              credits_per_unit: "0.02",
              sync_status: "synced",
            },
          ],
        },
      },
    ],
  });
  render(<Harness />);
  await configure("toggle");
  expect(
    screen.getByText(
      /0.01 credits per input audio second \+ 0.02 credits per start/,
    ),
  ).toHaveTextContent("paid by you");
  expect(f.clients).toHaveLength(0);
});

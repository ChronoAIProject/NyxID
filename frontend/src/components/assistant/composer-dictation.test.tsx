import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { voiceOption } from "@/test/voice-fixture";
import { useComposerDictation } from "./composer-dictation";
import { ApiError } from "@/lib/api-client";
const f = vi.hoisted(() => ({
  get: vi.fn(),
  permission: Promise.resolve() as Promise<void>,
  clients: [] as {
    callbacks: {
      state: (value: string) => void;
      text: (value: unknown) => void;
      error: (value: string) => void;
    };
    start: ReturnType<typeof vi.fn>;
    stop: ReturnType<typeof vi.fn>;
    cancel: ReturnType<typeof vi.fn>;
  }[],
}));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  api: { get: f.get },
}));
vi.mock("@/lib/assistant-dictation-client", () => ({
  AssistantDictationClient: class {
    callbacks: (typeof f.clients)[number]["callbacks"];
    start = vi.fn(async (prepare: () => Promise<unknown>) => {
      this.callbacks.state("requesting");
      try {
        await f.permission;
        await prepare();
      } catch {
        this.callbacks.error("Preparation failed");
        this.callbacks.state("review");
      }
    });
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
  threadId,
}: {
  scope?: string;
  disabled?: boolean;
  enabled?: boolean;
  threadId?: string;
}) {
  const d = useComposerDictation({
    scope,
    disabled,
    enabled,
    getThread,
    threadId,
    begin,
    insert,
    reviewClosed: closed,
  });
  return (
    <div data-testid="capture-state" data-capturing={d.captureActive}>
      {d.button}
      {d.review}
    </div>
  );
}
const mediaEvents = new EventTarget();
let devices: { deviceId: string; label: string; kind: string }[],
  getUserMedia: ReturnType<typeof vi.fn>,
  stopTrack: ReturnType<typeof vi.fn>;
beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
  f.clients = [];
  f.permission = Promise.resolve();
  f.get.mockResolvedValue({ options: [voiceOption] });
  getThread.mockResolvedValue("thread-A");
  devices = [
    {
      deviceId: "built-in",
      label: "MacBook Air Microphone",
      kind: "audioinput",
    },
  ];
  stopTrack = vi.fn();
  const track = Object.assign(new EventTarget(), { stop: stopTrack });
  getUserMedia = vi.fn().mockResolvedValue({
    getTracks: () => [track],
    getAudioTracks: () => [track],
  });
  vi.stubGlobal("navigator", {
    mediaDevices: {
      getUserMedia,
      enumerateDevices: vi.fn(async () => devices),
      addEventListener: mediaEvents.addEventListener.bind(mediaEvents),
      removeEventListener: mediaEvents.removeEventListener.bind(mediaEvents),
    },
  });
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
function configure() {
  fireEvent.click(screen.getByRole("button", { name: "Configure dictation" }));
}
it("overlaps existing-thread option discovery with permission without preparing a draft early", async () => {
  let grant!: () => void;
  f.permission = new Promise<void>((resolve) => {
    grant = resolve;
  });
  render(<Harness threadId="thread-A" />);
  fireEvent.click(screen.getByRole("button", { name: "Dictate a message" }));
  try {
    expect(f.get).toHaveBeenCalledWith(
      "/assistant/nyxagent/dictation/options?conversation_id=thread-A",
    );
    expect(getThread).not.toHaveBeenCalled();
  } finally {
    await act(async () => grant());
  }
  await waitFor(() => expect(getThread).toHaveBeenCalledOnce());
  expect(f.get).toHaveBeenCalledOnce();
});
it("a denied permission consumes no draft and safely ignores failed early discovery", async () => {
  let deny!: (error: Error) => void;
  f.permission = new Promise<void>((_resolve, reject) => {
    deny = reject;
  });
  f.get.mockRejectedValueOnce(new Error("Offline"));
  render(<Harness threadId="thread-A" />);
  fireEvent.click(screen.getByRole("button", { name: "Dictate a message" }));
  await act(async () => {});
  expect(f.get).toHaveBeenCalledOnce();
  await act(async () => deny(new Error("Denied")));
  expect(getThread).not.toHaveBeenCalled();
  expect(
    screen.getByRole("button", { name: "Dictate a message" }),
  ).toBeInTheDocument();
});
it("cancel while permission is pending fences early discovery and late preparation", async () => {
  let grant!: () => void;
  f.permission = new Promise<void>((resolve) => {
    grant = resolve;
  });
  render(<Harness threadId="thread-A" />);
  fireEvent.click(screen.getByRole("button", { name: "Dictate a message" }));
  fireEvent.click(screen.getByRole("button", { name: "Cancel dictation" }));
  await act(async () => grant());
  expect(getThread).not.toHaveBeenCalled();
  expect(screen.queryByText("Preparation failed")).not.toBeInTheDocument();
});
it("crossing between trigger and popup keeps settings open; leaving closes and stops the microphone test", async () => {
  render(<Harness />);
  configure();
  await screen.findByText("MacBook Air Microphone");
  fireEvent.click(screen.getByRole("button", { name: "Test microphone" }));
  await act(async () => {});
  vi.useFakeTimers();
  const trigger = screen.getByRole("button", { name: "Configure dictation" }).parentElement!;
  const popup = screen.getByRole("dialog");
  fireEvent.mouseLeave(trigger);
  act(() => vi.advanceTimersByTime(100));
  fireEvent.mouseEnter(popup);
  act(() => vi.advanceTimersByTime(300));
  expect(popup).toBeInTheDocument();
  expect(stopTrack).not.toHaveBeenCalled();
  fireEvent.mouseLeave(popup);
  act(() => vi.advanceTimersByTime(100));
  fireEvent.mouseEnter(trigger);
  act(() => vi.advanceTimersByTime(300));
  expect(popup).toBeInTheDocument();
  fireEvent.mouseLeave(trigger);
  act(() => vi.advanceTimersByTime(200));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(stopTrack).toHaveBeenCalledOnce();
  expect(f.clients).toHaveLength(0);
});
it("a pending mouse-out cannot close settings in a different draft", async () => {
  const { rerender } = render(<Harness />);
  configure();
  await screen.findByText("MacBook Air Microphone");
  vi.useFakeTimers();
  fireEvent.mouseLeave(screen.getByRole("dialog"));
  act(() => vi.advanceTimersByTime(100));
  rerender(<Harness scope="actor:draft-B" />);
  configure();
  await act(async () => vi.advanceTimersByTime(300));
  expect(screen.getByRole("dialog")).toBeInTheDocument();
});
it.each(["pointer", "keyboard"])(
  "main mic first and subsequent hold %s release survives permission-pending rerender",
  async (kind) => {
    render(<Harness />);
    configure();
    fireEvent.click(screen.getByRole("switch", { name: "Hold to record" }));
    const press = () => {
      const mic = screen.getByRole("button", { name: "Dictate a message" });
      if (kind === "pointer")
        fireEvent.pointerDown(mic, {
          pointerId: 7,
          button: 0,
          pointerType: "touch",
        });
      else fireEvent.keyDown(mic, { key: " " });
    };
    const release = () =>
      kind === "pointer"
        ? fireEvent.pointerUp(window, { pointerId: 7, button: 0 })
        : fireEvent.keyUp(window, { key: " " });
    press();
    expect(f.clients).toHaveLength(1);
    expect(begin).toHaveBeenCalledOnce();
    expect(
      screen.getByRole("button", { name: "Stop dictation" }),
    ).toBeInTheDocument();
    release();
    expect(f.clients[0]!.stop).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "Cancel dictation" }));
    press();
    expect(f.clients).toHaveLength(2);
    release();
    expect(f.clients[1]!.stop).toHaveBeenCalledOnce();
    await act(async () => {});
  },
);
it("direct click reviews final text, never inserts interim, and Escape preserves original draft", async () => {
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "Dictate a message" }));
  expect(f.clients).toHaveLength(1);
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
  await act(async () => {});
});
it("account/draft rebind clears authority and fences old callbacks before next start", async () => {
  const { rerender } = render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "Dictate a message" }));
  const old = f.clients[0]!;
  rerender(<Harness scope="other:draft-B" />);
  expect(old.cancel).toHaveBeenCalled();
  act(() => {
    old.callbacks.state("capturing");
    old.callbacks.text({ final: "leak", interim: "" });
  });
  expect(screen.queryByText("leak")).not.toBeInTheDocument();
  getThread.mockResolvedValue("thread-B");
  fireEvent.click(screen.getByRole("button", { name: "Dictate a message" }));
  await waitFor(() => expect(getThread).toHaveBeenLastCalledWith());
  expect(f.clients[1]!.start).toHaveBeenCalledWith(
    expect.any(Function),
    undefined,
    undefined,
  );
  await act(async () => {});
});
it.each(["disabled", "enabled"])(
  "loss of %s eligibility cancels without restoring stale focus",
  async (which) => {
    const { rerender } = render(<Harness />);
    fireEvent.click(screen.getByRole("button", { name: "Dictate a message" }));
    rerender(
      <Harness disabled={which === "disabled"} enabled={which !== "enabled"} />,
    );
    expect(f.clients[0]!.cancel).toHaveBeenCalled();
    expect(screen.queryByLabelText("Dictation review")).not.toBeInTheDocument();
    expect(closed).not.toHaveBeenCalled();
    await act(async () => {});
  },
);
it("only the chevron opens dynamic settings; hover does not request permission or write a draft, and the switch persists immediately", async () => {
  localStorage.setItem("nyxid:dictation-mode:actor", "hold");
  render(<Harness />);
  fireEvent.mouseEnter(
    screen.getByRole("button", { name: "Dictate a message" }),
  );
  fireEvent.focus(screen.getByRole("button", { name: "Dictate a message" }));
  expect(screen.queryByRole("switch", { name: "Hold to record" })).not.toBeInTheDocument();
  fireEvent.mouseEnter(screen.getByRole("button", { name: "Configure dictation" }));
  expect(screen.queryByRole("switch", { name: "Hold to record" })).not.toBeInTheDocument();
  fireEvent.mouseMove(screen.getByRole("button", { name: "Configure dictation" }), { movementX: 1 });
  expect(screen.getByRole("switch", { name: "Hold to record" })).toBeChecked();
  expect(
    await screen.findByRole("button", { name: "MacBook Air Microphone" }),
  ).toHaveAttribute("aria-pressed", "true");
  expect(getUserMedia).not.toHaveBeenCalled();
  expect(getThread).not.toHaveBeenCalled();
  expect(f.get).not.toHaveBeenCalled();
  expect(
    screen.queryByRole("button", { name: "Start dictation" }),
  ).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("switch", { name: "Hold to record" }));
  expect(localStorage.getItem("nyxid:dictation-mode:actor")).toBe("toggle");
});
it("live devicechange updates labels; selected capture applies immediately, unplug releases it, outside blur closes test", async () => {
  render(<Harness />);
  configure();
  await screen.findByText("MacBook Air Microphone");
  devices.push({
    deviceId: "usb",
    label: "USB microphone",
    kind: "audioinput",
  });
  act(() => mediaEvents.dispatchEvent(new Event("devicechange")));
  fireEvent.click(
    await screen.findByRole("button", { name: "USB microphone" }),
  );
  expect(getUserMedia).toHaveBeenCalledWith(
    expect.objectContaining({
      audio: expect.objectContaining({ deviceId: { exact: "usb" } }),
    }),
  );
  expect(localStorage.getItem("nyxid:microphone-device:actor")).toBe("usb");
  await act(async () => {});
  devices = devices.filter((d) => d.deviceId !== "usb");
  act(() => mediaEvents.dispatchEvent(new Event("devicechange")));
  expect(await screen.findByRole("alert")).toHaveTextContent("disconnected");
  expect(stopTrack).toHaveBeenCalledOnce();
  expect(
    screen.getByRole("button", { name: "MacBook Air Microphone" }),
  ).toHaveAttribute("aria-pressed", "false");
  fireEvent.click(
    screen.getByRole("button", { name: "MacBook Air Microphone" }),
  );
  await act(async () => {});
  // A late enumeration started by the prior USB selection cannot close the
  // replacement preview after the selected USB device has been unplugged.
  expect(stopTrack).toHaveBeenCalledTimes(1);
  expect(
    screen.getByRole("button", { name: "Stop microphone test" }),
  ).toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  fireEvent.blur(window);
  expect(stopTrack).toHaveBeenCalledTimes(2);
});
it("404 leaves device settings usable without claiming account/billing failure", async () => {
  f.get.mockRejectedValue(
    new ApiError(404, {
      error: "not_found",
      error_code: 404,
      message: "route missing",
    }),
  );
  render(<Harness threadId="thread-A" />);
  configure();
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "unavailable on the connected server",
  );
  fireEvent.click(screen.getByRole("button", { name: "Test microphone" }));
  expect(getUserMedia).toHaveBeenCalledOnce();
  await act(async () => {});
  fireEvent.click(screen.getByRole("button", { name: "Dictate a message" }));
  await waitFor(() =>
    expect(screen.getByRole("alert")).toHaveTextContent(
      "unavailable on the connected server",
    ),
  );
  expect(stopTrack).toHaveBeenCalledOnce();
  expect(screen.queryByLabelText("Dictation review")).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Configure dictation" }),
  ).toBeEnabled();
});
it("keeps authored duration and start prices in settings Credit usage, outside dictation review", async () => {
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
  render(<Harness threadId="thread-A" />);
  configure();
  const price = await screen.findByText(
    /0.01 credits per input audio second \+ 0.02 credits per start/,
  );
  expect(price).not.toBeVisible();
  fireEvent.click(screen.getByText("Credit usage"));
  expect(price).toHaveTextContent("paid by you");
  expect(f.clients).toHaveLength(0);
  expect(getUserMedia).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Dictate a message" }));
  await act(async () => {});
  for (const stage of ["capturing", "finalizing", "review"]) {
    act(() => {
      f.clients[0]!.callbacks.text({ final: "Keep my speech", interim: "" });
      f.clients[0]!.callbacks.state(stage);
    });
    expect(screen.getByLabelText("Dictation review")).toHaveTextContent(
      "Keep my speech",
    );
    expect(
      screen.queryByText(/credits per input audio second/),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/paid by you/)).not.toBeInTheDocument();
  }
});

it("failed capture with real partial text keeps review without implying microphone capture", async () => {
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "Dictate a message" }));
  act(() => {
    f.clients[0]!.callbacks.text({ final: "Retained speech", interim: "" });
    f.clients[0]!.callbacks.error("Connection ended");
    f.clients[0]!.callbacks.state("review");
  });
  expect(screen.getByLabelText("Dictation review")).toHaveTextContent(
    "Retained speech",
  );
  expect(
    screen.getByRole("button", { name: "Insert dictation" }),
  ).toBeEnabled();
  expect(screen.getByTestId("capture-state")).toHaveAttribute(
    "data-capturing",
    "false",
  );
  await act(async () => {});
});

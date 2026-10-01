import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { MachineDesktopPanel } from "./machine-desktop-panel";
vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({ data: { node_ws_url: "https://nyxid.example" } }),
}));
vi.mock("@/hooks/use-machines", () => ({
  useMachineDesktops: () => ({ data: [] }),
}));
class Socket {
  static OPEN = 1;
  static instances: Socket[] = [];
  readyState = 1;
  bufferedAmount = 0;
  binaryType = "";
  onmessage: ((event: { data: string | ArrayBuffer }) => void) | null = null;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  send = vi.fn();
  close = vi.fn();
  readonly url: string;
  constructor(url: string) {
    this.url = url;
    Socket.instances.push(this);
  }
  message(value: unknown) {
    this.onmessage?.({ data: JSON.stringify(value) });
  }
}
beforeEach(() => {
  Socket.instances = [];
  vi.stubGlobal("WebSocket", Socket);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
    clearRect: vi.fn(),
  } as unknown as CanvasRenderingContext2D);
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});
it("watches, requests control, protects owner input, and hands back with a note", async () => {
  const view = render(
    <MachineDesktopPanel
      nodeId="node"
      conversationId="thread"
      reason="Please sign in"
    />,
  );
  const socket = Socket.instances[0]!;
  expect(screen.getByText("Connecting…")).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Please sign in");
  act(() =>
    socket.message({
      type: "connected",
      session_id: "12345678-1234-4234-8234-123456789abc",
      controller: "agent",
    }),
  );
  const canvas = screen.getByLabelText(/Machine screen/);
  fireEvent.keyDown(canvas, { key: "x" });
  expect(socket.send).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Take control" }));
  expect(JSON.parse(socket.send.mock.lastCall![0])).toEqual({
    type: "take_control",
  });
  act(() =>
    socket.message({ type: "state", controller: "owner", controls: true }),
  );
  expect(screen.getByText("You are in control")).toBeInTheDocument();
  fireEvent.keyDown(canvas, { key: "ArrowLeft" });
  const frame = socket.send.mock.lastCall![0] as ArrayBuffer;
  expect(
    JSON.parse(new TextDecoder().decode(new Uint8Array(frame).slice(30))),
  ).toEqual({ tool: "press_key", arguments: { key: "LEFT" } });
  fireEvent.change(screen.getByLabelText("Hand-back note"), {
    target: { value: "logged in" },
  });
  await act(async () =>
    fireEvent.submit(
      screen.getByRole("button", { name: "Hand back" }).closest("form")!,
    ),
  );
  expect(JSON.parse(socket.send.mock.lastCall![0])).toEqual({
    type: "hand_back",
    note: "logged in",
  });
  act(() =>
    socket.message({ type: "state", controller: "agent", controls: false }),
  );
  const sent = socket.send.mock.calls.length;
  fireEvent.keyDown(canvas, { key: "x" });
  expect(socket.send).toHaveBeenCalledTimes(sent);
  fireEvent.click(screen.getByRole("button", { name: "Stop" }));
  expect(JSON.parse(socket.send.mock.lastCall![0])).toEqual({ type: "stop" });
  view.unmount();
  expect(socket.close).toHaveBeenCalledOnce();
});
it("clears authority after disconnect and reconnects with a fresh socket", () => {
  render(<MachineDesktopPanel nodeId="node" />);
  const socket = Socket.instances[0]!;
  act(() =>
    socket.message({
      type: "connected",
      session_id: "12345678-1234-4234-8234-123456789abc",
      controller: "owner",
      controls: true,
    }),
  );
  act(() => socket.onclose?.());
  expect(screen.getByRole("alert")).toHaveTextContent("disconnected");
  expect(screen.getByRole("button", { name: "Take control" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
  expect(Socket.instances).toHaveLength(2);
  expect(socket.close).toHaveBeenCalledOnce();
});
it("paints ordered dirty rectangles and requests a full frame when a base is missing", async () => {
  const drawImage = vi.fn();
  vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue({
    drawImage,
    clearRect: vi.fn(),
  } as unknown as CanvasRenderingContext2D);
  const close = vi.fn();
  const decode = vi
    .fn()
    .mockResolvedValueOnce({ width: 1280, height: 800, close })
    .mockResolvedValueOnce({ width: 64, height: 64, close });
  vi.stubGlobal("createImageBitmap", decode);
  const id = "12345678-1234-4234-8234-123456789abc";
  const packet = (sequence: bigint, base: bigint) => {
    const bytes = new Uint8Array(54 + 3);
    bytes.set([78, 89, 88, 77, 1, 0]);
    const hex = id.replaceAll("-", "");
    for (let n = 0; n < 16; n++)
      bytes[6 + n] = Number.parseInt(hex.slice(n * 2, n * 2 + 2), 16);
    const view = new DataView(bytes.buffer);
    view.setBigUint64(22, sequence);
    bytes.set([78, 89, 88, 68], 30);
    const rectangle =
      base === 0n ? [1280, 800, 0, 0, 1280, 800] : [1280, 800, 64, 128, 64, 64];
    rectangle.forEach((value, n) => view.setUint16(34 + n * 2, value));
    view.setBigUint64(46, base);
    return bytes.buffer;
  };
  render(<MachineDesktopPanel nodeId="node" />);
  const socket = Socket.instances[0]!;
  act(() =>
    socket.message({ type: "connected", session_id: id, controller: "agent" }),
  );
  await act(async () => {
    socket.onmessage?.({ data: packet(1n, 0n) });
    socket.onmessage?.({ data: packet(2n, 1n) });
  });
  expect(decode).toHaveBeenCalledTimes(2);
  expect(drawImage).toHaveBeenCalledWith(
    expect.objectContaining({ width: 64 }),
    64,
    128,
  );
  expect(close).toHaveBeenCalledTimes(2);
  await act(async () => socket.onmessage?.({ data: packet(4n, 3n) }));
  expect(decode).toHaveBeenCalledTimes(2);
  expect(JSON.parse(socket.send.mock.lastCall![0])).toEqual({
    type: "refresh_frame",
  });
});

it("remembers a collapsed strip per conversation and keeps its Stop control", () => {
  localStorage.clear();
  const view = render(
    <MachineDesktopPanel
      nodeId="node"
      conversationId="collapse-thread"
      name="Work machine"
      turnActive
    />,
  );
  act(() =>
    Socket.instances[0]!.message({
      type: "connected",
      session_id: "id",
      controller: "agent",
    }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Minimise desktop" }));
  expect(screen.getByRole("button", { name: "Show desktop" })).toHaveAttribute(
    "aria-expanded",
    "false",
  );
  expect(screen.getByLabelText(/Machine screen/)).not.toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Stop" }));
  expect(JSON.parse(Socket.instances[0]!.send.mock.lastCall![0])).toEqual({
    type: "stop",
  });
  view.unmount();
  const reopened = render(
    <MachineDesktopPanel nodeId="node" conversationId="collapse-thread" />,
  );
  expect(
    screen.getByRole("button", { name: "Show desktop" }),
  ).toBeInTheDocument();
  reopened.unmount();
  render(<MachineDesktopPanel nodeId="node" conversationId="another-thread" />);
  expect(
    screen.getByRole("button", { name: "Minimise desktop" }),
  ).toBeInTheDocument();
  localStorage.clear();
});

it("collapses after a turn ends without recent machine activity", () => {
  vi.useFakeTimers();
  const view = render(
    <MachineDesktopPanel
      nodeId="node"
      conversationId="idle-thread"
      turnActive
    />,
  );
  act(() =>
    Socket.instances[0]!.message({
      type: "connected",
      session_id: "id",
      controller: "agent",
    }),
  );
  view.rerender(
    <MachineDesktopPanel
      nodeId="node"
      conversationId="idle-thread"
      turnActive={false}
    />,
  );
  act(() => vi.advanceTimersByTime(3001));
  expect(
    screen.getByRole("button", { name: "Show desktop" }),
  ).toBeInTheDocument();
  view.unmount();
  vi.useRealTimers();
  localStorage.clear();
});

it("switches capture sockets by display and keeps owner input on its controller", () => {
  const view = render(<MachineDesktopPanel nodeId="node" />);
  const secure = Socket.instances[0]!;
  expect(secure.url).not.toContain("display=dev");
  expect(
    screen.getByRole("button", { name: "Secure browser" }),
  ).toHaveAttribute("aria-pressed", "true");
  fireEvent.click(screen.getByRole("button", { name: "Dev browser" }));
  expect(secure.close).toHaveBeenCalledOnce();
  const dev = Socket.instances[1]!;
  expect(dev.url).toContain("display=dev");
  act(() =>
    dev.message({
      type: "connected",
      session_id: "12345678-1234-4234-8234-123456789abc",
      controller: "owner",
      controls: true,
    }),
  );
  expect(screen.getByRole("button", { name: "Secure browser" })).toBeDisabled();
  fireEvent.keyDown(screen.getByLabelText(/Machine screen/), {
    key: "ArrowLeft",
  });
  expect(dev.send).toHaveBeenCalledOnce();
  expect(secure.send).not.toHaveBeenCalled();
  view.unmount();
});

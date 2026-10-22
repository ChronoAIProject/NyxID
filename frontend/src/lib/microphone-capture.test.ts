import { afterEach, expect, it, vi } from "vitest";
import {
  MicrophoneCapture,
  PreparedMicrophone,
  microphoneError,
} from "./microphone-capture";
afterEach(() => vi.unstubAllGlobals());
function media() {
  const track = new EventTarget() as EventTarget & {
    stop: ReturnType<typeof vi.fn>;
  };
  track.stop = vi.fn();
  return {
    track,
    stream: {
      getTracks: () => [track],
      getAudioTracks: () => [track],
    } as unknown as MediaStream,
  };
}
it("requests intentionally, releases late grants on cancellation, and excludes concurrent owners", async () => {
  let grant!: (s: MediaStream) => void;
  const get = vi.fn(
    () =>
      new Promise<MediaStream>((resolve) => {
        grant = resolve;
      }),
  );
  vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: get } });
  const first = new MicrophoneCapture();
  expect(get).toHaveBeenCalledOnce();
  expect(() => new MicrophoneCapture()).toThrow();
  const rejected = expect(first.ready).rejects.toMatchObject({
    name: "AbortError",
  });
  first.close();
  const m = media();
  grant(m.stream);
  await rejected;
  expect(m.track.stop).toHaveBeenCalledOnce();
  first.close();
  expect(m.track.stop).toHaveBeenCalledOnce();
  const second = new MicrophoneCapture();
  second.close();
  grant(media().stream);
  await expect(second.ready).rejects.toMatchObject({ name: "AbortError" });
});
it("revocation cleans tracks and releases ownership exactly once", async () => {
  const m = media();
  vi.stubGlobal("navigator", {
    mediaDevices: { getUserMedia: vi.fn().mockResolvedValue(m.stream) },
  });
  const revoked = vi.fn();
  const capture = new MicrophoneCapture(revoked);
  await capture.ready;
  m.track.dispatchEvent(new Event("ended"));
  capture.close();
  expect(revoked).toHaveBeenCalledOnce();
  expect(m.track.stop).toHaveBeenCalledOnce();
  const next = new MicrophoneCapture();
  await next.ready;
  next.close();
});
it("sync and async permission failures release ownership and sanitize messages", async () => {
  const get = vi
    .fn()
    .mockImplementationOnce(() => {
      throw new DOMException("secret", "NotAllowedError");
    })
    .mockRejectedValueOnce(new DOMException("secret", "NotReadableError"))
    .mockResolvedValue(media().stream);
  vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: get } });
  expect(() => new MicrophoneCapture()).toThrow();
  const failed = new MicrophoneCapture();
  await expect(failed.ready).rejects.toMatchObject({
    name: "NotReadableError",
  });
  const next = new MicrophoneCapture();
  await next.ready;
  next.close();
  expect(microphoneError(new Error("secret"))).not.toContain("secret");
  expect(
    microphoneError(new DOMException("secret", "NotFoundError")),
  ).toContain("No microphone");
});

it("selected microphone uses exact device and prepared ownership transfers without a second capture", async () => {
  const m = media();
  const get = vi.fn().mockResolvedValue(m.stream);
  vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: get } });
  const prepared = new PreparedMicrophone("usb-device");
  expect(get).toHaveBeenCalledWith(
    expect.objectContaining({
      audio: expect.objectContaining({ deviceId: { exact: "usb-device" } }),
    }),
  );
  await prepared.ready;
  prepared.handoff();
  expect(prepared.isTransferred).toBe(true);
  prepared.close();
  prepared.close();
  expect(m.track.stop).toHaveBeenCalledOnce();
  expect(prepared.isClosed).toBe(true);
});

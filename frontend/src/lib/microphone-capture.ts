/** One ephemeral capture owner, including pending browser permission prompts. */
let owner: symbol | undefined;
export type AudioActivity = {
  input: number;
  output: number;
  bands: number[];
  playbackMs: number;
};
export const silentActivity = (): AudioActivity => ({
  input: 0,
  output: 0,
  bands: Array<number>(24).fill(0),
  playbackMs: 0,
});

export class MicrophoneCapture {
  readonly ready: Promise<MediaStream>;
  private readonly identity = Symbol("capture");
  private closed = false;
  private stream?: MediaStream;
  private revoked: () => void;
  constructor(revoked: () => void = () => {}) {
    this.revoked = revoked;
    if (owner) throw new Error("End the current microphone session first.");
    if (!navigator.mediaDevices?.getUserMedia)
      throw new Error(
        "Microphone capture is unavailable. Use a supported browser over HTTPS.",
      );
    owner = this.identity;
    // Called synchronously by an intentional click/press, before network awaits.
    let request: Promise<MediaStream>;
    try {
      request = navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
        video: false,
      });
    } catch (error) {
      this.close();
      throw error;
    }
    this.ready = request
      .then((stream) => {
        if (this.closed) {
          stream.getTracks().forEach((track) => track.stop());
          throw new DOMException("Microphone request cancelled", "AbortError");
        }
        this.stream = stream;
        stream
          .getAudioTracks()
          .forEach((track) => track.addEventListener?.("ended", this.onEnded));
        return stream;
      })
      .catch((error: unknown) => {
        this.close();
        throw error;
      });
  }
  private onEnded = () => {
    if (!this.closed) {
      this.close();
      this.revoked();
    }
  };
  close() {
    if (this.closed) return;
    this.closed = true;
    this.stream?.getTracks().forEach((track) => {
      track.removeEventListener?.("ended", this.onEnded);
      track.stop();
    });
    this.stream = undefined;
    if (owner === this.identity) owner = undefined;
  }
}

/** Analysis only. Never connect microphone or remote streams to the speakers. */
export class AudioActivityMeter {
  private context?: AudioContext;
  private input?: AnalyserNode;
  private output?: AnalyserNode;
  private sources: MediaStreamAudioSourceNode[] = [];
  private timer?: ReturnType<typeof setInterval>;
  private update: (activity: AudioActivity) => void;
  private playback: () => number;
  constructor(
    update: (activity: AudioActivity) => void,
    playback: () => number = () => 0,
  ) {
    this.update = update;
    this.playback = playback;
    if (typeof AudioContext === "undefined") return;
    try {
      this.context = new AudioContext();
      void this.context.resume().catch(() => {});
    } catch {
      /* Capture remains available without visual analysis. */
    }
  }
  watch(stream: MediaStream, direction: "input" | "output") {
    if (!this.context) return;
    const source = this.context.createMediaStreamSource(stream);
    const analyser = this.context.createAnalyser();
    analyser.fftSize = 256;
    source.connect(analyser);
    this.sources.push(source);
    this[direction] = analyser;
    this.timer ??= setInterval(() => this.sample(), 80);
  }
  private measure(analyser?: AnalyserNode) {
    if (!analyser || this.context?.state !== "running")
      return { level: 0, bands: Array<number>(24).fill(0) };
    const data = new Float32Array(analyser.fftSize);
    analyser.getFloatTimeDomainData(data);
    const rms = Math.sqrt(
      data.reduce((sum, value) => sum + value * value, 0) / data.length,
    );
    return {
      level: Math.min(1, rms * 4),
      bands: Array.from({ length: 24 }, (_, i) =>
        Math.min(
          1,
          Math.max(...data.subarray(i * 10, i * 10 + 10).map(Math.abs)) * 3,
        ),
      ),
    };
  }
  private sample() {
    const input = this.measure(this.input);
    const output = this.measure(this.output);
    this.update({
      input: input.level,
      output: output.level,
      bands: input.bands,
      playbackMs: this.playback(),
    });
  }
  resume() {
    return this.context?.resume();
  }
  close() {
    clearInterval(this.timer);
    this.sources.forEach((source) => source.disconnect());
    this.input?.disconnect();
    this.output?.disconnect();
    void this.context?.close().catch(() => {});
    this.context = undefined;
    this.update(silentActivity());
  }
}

export function microphoneError(error: unknown): string {
  const name = error instanceof DOMException ? error.name : "";
  if (name === "NotAllowedError" || name === "SecurityError")
    return "Microphone access was denied. Allow it in your browser settings, then try again.";
  if (name === "NotFoundError" || name === "OverconstrainedError")
    return "No microphone was found. Connect a microphone and try again.";
  if (name === "NotReadableError" || name === "AbortError")
    return "The microphone is busy or was disconnected. Close other microphone apps and try again.";
  return "Microphone capture is unavailable. End other microphone sessions or use a supported browser over HTTPS.";
}

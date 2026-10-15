import workletUrl from "./grok-audio-worklet.js?url&no-inline";

/** Reuse the bounded mono 24 kHz encoder. No microphone monitoring/playback. */
export class DictationPcm {
  private context?: AudioContext;
  private node?: AudioWorkletNode;
  private source?: MediaStreamAudioSourceNode;
  private closed = false;
  constructor() {
    this.context = new AudioContext();
    void this.context.resume().catch(() => {});
  }
  async start(stream: MediaStream, pcm: (bytes: ArrayBuffer) => void) {
    const context = this.context!;
    await context.resume();
    await context.audioWorklet.addModule(workletUrl);
    if (this.closed) return;
    const node = new AudioWorkletNode(context, "nyx-grok-audio", {
      numberOfInputs: 1,
      numberOfOutputs: 1,
      outputChannelCount: [1],
    });
    this.node = node;
    this.source = context.createMediaStreamSource(stream);
    this.source.connect(node);
    const silence = context.createGain();
    silence.gain.value = 0;
    node.connect(silence).connect(context.destination);
    node.port.onmessage = ({ data }) => {
      if (!this.closed && data.type === "pcm" && data.generation === 1)
        pcm(data.bytes as ArrayBuffer);
    };
    node.port.postMessage({ type: "capture", enabled: true, generation: 1 });
  }
  close() {
    if (this.closed) return;
    this.closed = true;
    this.node?.port.postMessage({
      type: "capture",
      enabled: false,
      generation: 2,
    });
    this.node?.port.close();
    this.node?.disconnect();
    this.source?.disconnect();
    void this.context?.close().catch(() => {});
  }
}

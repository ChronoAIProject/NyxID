import workletUrl from "./grok-audio-worklet.js?url&no-inline";

/** PCM playback enters a WebRTC receive track for the browser AEC reference. */
export class GrokAudio {
  private context?: AudioContext;
  private worklet?: AudioWorkletNode;
  private source?: MediaStreamAudioSourceNode;
  private sender?: RTCPeerConnection;
  private receiver?: RTCPeerConnection;
  private marks: { end: number; at: number }[] = [];
  private baseline = 0;
  private played = 0;
  private closed = false;
  private capturing = false;
  private captureGeneration = 0;
  private playbackGeneration = 0;
  private audio: HTMLAudioElement;
  private pcm: (bytes: ArrayBuffer) => void;
  private failed: () => void;
  constructor(
    audio: HTMLAudioElement,
    pcm: (bytes: ArrayBuffer) => void,
    failed: () => void,
  ) {
    this.audio = audio;
    this.pcm = pcm;
    this.failed = failed;
  }
  async start(media: MediaStream) {
    const context = new AudioContext();
    this.context = context;
    await context.resume();
    await context.audioWorklet.addModule(workletUrl);
    if (this.closed) return;
    const worklet = new AudioWorkletNode(context, "nyx-grok-audio", {
      numberOfInputs: 1,
      numberOfOutputs: 1,
      outputChannelCount: [1],
    });
    this.worklet = worklet;
    this.source = context.createMediaStreamSource(media);
    this.source.connect(worklet);
    const destination = context.createMediaStreamDestination();
    worklet.connect(destination); // Never connect to context.destination (speakers).
    const sender = new RTCPeerConnection({ iceServers: [] });
    const receiver = new RTCPeerConnection({ iceServers: [] });
    this.sender = sender;
    this.receiver = receiver;
    const pendingSender: RTCIceCandidate[] = [];
    const pendingReceiver: RTCIceCandidate[] = [];
    sender.onicecandidate = ({ candidate }) => {
      if (!candidate || this.closed) return;
      if (receiver.remoteDescription)
        void receiver.addIceCandidate(candidate).catch(this.failed);
      else pendingReceiver.push(candidate);
    };
    receiver.onicecandidate = ({ candidate }) => {
      if (!candidate || this.closed) return;
      if (sender.remoteDescription)
        void sender.addIceCandidate(candidate).catch(this.failed);
      else pendingSender.push(candidate);
    };
    receiver.ontrack = ({ track, streams }) => {
      if (this.closed) return;
      this.audio.srcObject = streams[0] ?? new MediaStream([track]);
      this.baseline = this.audio.currentTime - context.currentTime;
      void this.audio.play().catch(() => {
        /* Resume audio is exposed in the panel. */
      });
    };
    receiver.onconnectionstatechange = () => {
      if (receiver.connectionState === "failed") this.failed();
    };
    destination.stream
      .getTracks()
      .forEach((t) => sender.addTrack(t, destination.stream));
    worklet.port.onmessage = ({ data }) => {
      if (this.closed) return;
      if (
        data.type === "pcm" &&
        this.capturing &&
        data.generation === this.captureGeneration
      )
        this.pcm(data.bytes as ArrayBuffer);
      if (data.type === "played" && data.generation === this.playbackGeneration)
        this.marks.push({
          end: data.end_ms as number,
          at: (data.at as number) + this.baseline + 0.2,
        });
      if (data.type === "overflow" || this.marks.length > 256) this.failed();
    };
    await sender.setLocalDescription(await sender.createOffer());
    if (this.closed) return;
    await receiver.setRemoteDescription(sender.localDescription!);
    for (const candidate of pendingReceiver)
      await receiver.addIceCandidate(candidate);
    await receiver.setLocalDescription(await receiver.createAnswer());
    if (this.closed) return;
    await sender.setRemoteDescription(receiver.localDescription!);
    for (const candidate of pendingSender)
      await sender.addIceCandidate(candidate);
  }
  capture(enabled: boolean) {
    if (this.capturing === enabled) return;
    this.capturing = enabled;
    this.worklet?.port.postMessage({
      type: "capture",
      enabled,
      generation: ++this.captureGeneration,
    });
  }
  enqueue(bytes: ArrayBuffer, end_ms: number) {
    this.worklet?.port.postMessage(
      { type: "audio", bytes, end_ms, generation: this.playbackGeneration },
      [bytes],
    );
  }
  flush() {
    ++this.playbackGeneration;
    this.worklet?.port.postMessage({ type: "flush" });
    this.marks = [];
  }
  watermark() {
    if (this.context?.state !== "running" || this.audio.paused)
      return this.played;
    while (this.marks[0] && this.marks[0].at <= this.audio.currentTime) {
      this.played = Math.max(this.played, this.marks.shift()!.end);
    }
    return this.played;
  }
  async resume() {
    await this.context?.resume();
  }
  close() {
    if (this.closed) return;
    this.closed = true;
    this.capture(false);
    this.source?.disconnect();
    this.worklet?.disconnect();
    this.worklet?.port.close();
    this.sender?.getSenders().forEach((s) => s.track?.stop());
    this.sender?.close();
    this.receiver?.close();
    void this.context?.close();
    this.marks = [];
  }
}

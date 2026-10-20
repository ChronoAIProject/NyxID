/* global AudioWorkletProcessor, registerProcessor, sampleRate, currentTime */
// Ephemeral PCM only. No microphone monitoring and no recording.
class GrokAudioProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.capture = false;
    this.captureGeneration = 0;
    this.previous = 0;
    this.capturePosition = 0;
    this.packet = [];
    this.queue = [];
    this.queued = 0;
    this.playPosition = 0;
    this.port.onmessage = ({ data }) => {
      if (data.type === "capture") {
        this.capture = data.enabled;
        this.captureGeneration = data.generation;
        this.packet = [];
        this.capturePosition = 0;
      } else if (data.type === "flush") {
        this.queue = [];
        this.queued = 0;
        this.playPosition = 0;
      } else if (data.type === "audio") {
        const view = new DataView(data.bytes);
        const pcm = new Float32Array(view.byteLength / 2);
        for (let i = 0; i < pcm.length; i++)
          pcm[i] = view.getInt16(i * 2, true) / 32768;
        this.queued += pcm.length;
        if (this.queued > 24000 * 5) {
          this.queue = [];
          this.queued = 0;
          this.port.postMessage({ type: "overflow" });
          return;
        }
        this.queue.push({ pcm, end: data.end_ms, generation: data.generation });
      }
    };
  }
  process(inputs, outputs) {
    const input = inputs[0]?.[0];
    if (this.capture && input) {
      const step = sampleRate / 24000;
      while (this.capturePosition < input.length) {
        const at = Math.floor(this.capturePosition);
        const fraction = this.capturePosition - at;
        const before = at === 0 ? this.previous : input[at - 1];
        const value = before + (input[at] - before) * fraction;
        this.packet.push(Math.round(Math.max(-1, Math.min(1, value)) * 32767));
        this.capturePosition += step;
        if (this.packet.length === 480) {
          const bytes = new ArrayBuffer(960);
          const view = new DataView(bytes);
          this.packet.forEach((v, i) => view.setInt16(i * 2, v, true));
          this.port.postMessage(
            { type: "pcm", bytes, generation: this.captureGeneration },
            [bytes],
          );
          this.packet = [];
        }
      }
      this.capturePosition -= input.length;
      this.previous = input[input.length - 1];
    }
    const output = outputs[0]?.[0];
    if (!output) return true;
    for (let i = 0; i < output.length; i++) {
      const item = this.queue[0];
      if (!item) {
        output[i] = 0;
        continue;
      }
      const at = Math.floor(this.playPosition);
      const fraction = this.playPosition - at;
      const next = item.pcm[at + 1] ?? this.queue[1]?.pcm[0] ?? item.pcm[at];
      output[i] = item.pcm[at] + (next - item.pcm[at]) * fraction;
      this.playPosition += 24000 / sampleRate;
      while (this.queue[0] && this.playPosition >= this.queue[0].pcm.length) {
        const completed = this.queue.shift();
        this.playPosition -= completed.pcm.length;
        this.queued -= completed.pcm.length;
        this.port.postMessage({
          type: "played",
          generation: completed.generation,
          end_ms: completed.end,
          at: currentTime + i / sampleRate,
        });
      }
      if (!this.queue.length) this.playPosition = 0;
    }
    return true;
  }
}
registerProcessor("nyx-grok-audio", GrokAudioProcessor);

/** Ephemeral media only: no authentication, HTTP requests, storage, or app tools. */
export const VOICE_MEDIA_ENGINE = String.raw`
(() => {
  let generation = 0;
  let closed = true;
  let media, peer, sender, receiver, audio, context, worklet, source, destination;
  let captureGeneration = 0;
  let playbackGeneration = 0;
  let capturing = false;
  let speakerMuted = false;
  let baseline = 0;
  let played = 0;
  let marks = [];
  let pcmSequence = 0;
  let stage = 'runtime';
  const pendingPCM = new Set();
  const iceWaiters = new Set();
  const MAX_FRAME = 1920;
  const alive = (id) => !closed && id === generation;
  const post = (event) => window.ReactNativeWebView.postMessage(JSON.stringify(event));
  const emit = (event) => post({ ...event, generation });
  const atStage = (value) => { stage = value; emit({ type: 'diagnostic', stage, name: 'pending' }); };
  function candidateCounts(description) {
    const counts = { candidates: 0, host: 0, srflx: 0, relay: 0, mdns: 0, udp: 0, tcp: 0, ipv4: 0, ipv6: 0 };
    for (const line of (description?.sdp || '').split(/\r?\n/)) {
      if (!line.startsWith('a=candidate:')) continue;
      const parts = line.split(/\s+/);
      counts.candidates++;
      if (parts[2]?.toLowerCase() === 'udp') counts.udp++;
      if (parts[2]?.toLowerCase() === 'tcp') counts.tcp++;
      if (parts[7] === 'host') counts.host++;
      if (parts[7] === 'srflx') counts.srflx++;
      if (parts[7] === 'relay') counts.relay++;
      if (parts[4]?.endsWith('.local')) counts.mdns++;
      else if (parts[4]?.includes(':')) counts.ipv6++;
      else if (/^\d+\.\d+\.\d+\.\d+$/.test(parts[4] || '')) counts.ipv4++;
    }
    return counts;
  }
  const assertAlive = (id) => { if (!alive(id)) throw new Error('cancelled'); };
  function cleanup() {
    closed = true;
    capturing = false;
    for (const reject of iceWaiters) reject(new Error('cancelled'));
    iceWaiters.clear();
    media?.getTracks().forEach((track) => track.stop());
    media = undefined;
    source?.disconnect();
    worklet?.disconnect();
    worklet?.port.close();
    destination?.stream.getTracks().forEach((track) => track.stop());
    for (const connection of [peer, sender, receiver]) connection?.close();
    peer = sender = receiver = undefined;
    if (audio) { audio.pause(); audio.srcObject = null; audio.remove(); }
    audio = undefined;
    if (context) void context.close().catch(() => {});
    context = worklet = source = destination = undefined;
    marks = [];
    played = baseline = 0;
    pendingPCM.clear();
  }
  function fail(message) {
    if (closed) return;
    emit({ type: 'error', message });
    cleanup();
  }
  function playback() {
    if (context?.state === 'running' && audio && !audio.paused) {
      while (marks[0] && marks[0].at <= audio.currentTime) {
        played = Math.max(played, marks.shift().end);
      }
    }
    return {
      playback_ms: worklet ? played : Math.max(0, Math.floor((audio?.currentTime || 0) * 1000)),
      playback_audible: !!audio && !audio.paused && !audio.muted && audio.volume > 0 && !!audio.srcObject,
    };
  }
  function waitForICE(connection, id) {
    if (connection.iceGatheringState === 'complete') return Promise.resolve();
    return new Promise((resolve, reject) => {
      let finished = false;
      const finish = (error) => {
        if (finished) return;
        finished = true;
        clearTimeout(timer);
        connection.removeEventListener('icegatheringstatechange', changed);
        iceWaiters.delete(finish);
        error ? reject(error) : resolve();
      };
      const changed = () => {
        if (!alive(id)) finish(new Error('cancelled'));
        else if (connection.iceGatheringState === 'complete') finish();
      };
      const timer = setTimeout(() => finish(new Error('ice')), 5000);
      iceWaiters.add(finish);
      connection.addEventListener('icegatheringstatechange', changed);
      changed();
    });
  }
  function waitForConnected(connection, id) {
    if (connection.connectionState === 'connected') return Promise.resolve();
    return new Promise((resolve, reject) => {
      let finished = false;
      const finish = (error) => {
        if (finished) return;
        finished = true;
        clearTimeout(timer);
        connection.removeEventListener('connectionstatechange', changed);
        iceWaiters.delete(finish);
        error ? reject(error) : resolve();
      };
      const changed = () => {
        if (!alive(id)) finish(new Error('cancelled'));
        else if (connection.connectionState === 'connected') finish();
        else if (connection.connectionState === 'failed' || connection.connectionState === 'closed') finish(new Error('loopback'));
      };
      const timer = setTimeout(() => finish(new Error('loopback')), 5000);
      iceWaiters.add(finish);
      connection.addEventListener('connectionstatechange', changed);
      changed();
    });
  }
  function encode(bytes) {
    let binary = '';
    for (const byte of new Uint8Array(bytes)) binary += String.fromCharCode(byte);
    return btoa(binary);
  }
  function decode(value) {
    if (typeof value !== 'string' || value.length > 2560 || value.length % 4) throw new Error('frame');
    const binary = atob(value);
    if (!binary.length || binary.length > MAX_FRAME || binary.length % 2) throw new Error('frame');
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes.buffer;
  }
  async function grok(id, processor) {
    atStage('audio_context');
    if (!window.AudioWorkletNode || !window.AudioContext) throw new Error('unsupported');
    const localContext = new AudioContext();
    context = localContext;
    if (!localContext.audioWorklet) throw new Error('unsupported');
    await localContext.resume();
    assertAlive(id);
    atStage('audio_worklet');
    const url = URL.createObjectURL(new Blob([processor], { type: 'text/javascript' }));
    try { await localContext.audioWorklet.addModule(url); }
    finally { URL.revokeObjectURL(url); }
    assertAlive(id);
    const localWorklet = new AudioWorkletNode(localContext, 'nyx-grok-audio', {
      numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1],
    });
    worklet = localWorklet;
    source = localContext.createMediaStreamSource(media);
    source.connect(localWorklet);
    destination = localContext.createMediaStreamDestination();
    localWorklet.connect(destination);
    // Exactly the web audio path: receive-track playback supplies the AEC reference.
    // Never also connect to context.destination (that would play duplicate audio).
    const localSender = new RTCPeerConnection({ iceServers: [] });
    const localReceiver = new RTCPeerConnection({ iceServers: [] });
    sender = localSender;
    receiver = localReceiver;
    localSender.onconnectionstatechange = () => {
      if (alive(id)) emit({ type: 'diagnostic', stage: 'loopback_sender_state', name: localSender.connectionState });
    };
    localSender.oniceconnectionstatechange = () => {
      if (alive(id)) emit({ type: 'diagnostic', stage: 'loopback_sender_ice', name: localSender.iceConnectionState });
    };
    localReceiver.oniceconnectionstatechange = () => {
      if (alive(id)) emit({ type: 'diagnostic', stage: 'loopback_receiver_ice', name: localReceiver.iceConnectionState });
    };
    localReceiver.ontrack = ({ track, streams }) => {
      if (!alive(id)) return;
      audio.srcObject = streams[0] || new MediaStream([track]);
      baseline = audio.currentTime - localContext.currentTime;
      emit({ type: 'diagnostic', stage: 'loopback_receiver_track', name: 'ready' });
      void audio.play().catch(() => {});
    };
    localReceiver.onconnectionstatechange = () => {
      if (alive(id)) emit({ type: 'diagnostic', stage: 'loopback_receiver_state', name: localReceiver.connectionState });
      if (alive(id) && localReceiver.connectionState === 'failed') fail('Voice playback could not be verified.');
    };
    localWorklet.port.onmessage = ({ data }) => {
      if (!alive(id)) return;
      if (data.type === 'pcm' && capturing && data.generation === captureGeneration) {
        if (!(data.bytes instanceof ArrayBuffer) || data.bytes.byteLength !== 960 || pendingPCM.size >= 100) {
          fail('Voice audio connection is too slow. Please reconnect.'); return;
        }
        const sequence = ++pcmSequence;
        pendingPCM.add(sequence);
        emit({ type: 'pcm', sequence, bytes: encode(data.bytes) });
      }
      if (data.type === 'played' && data.generation === playbackGeneration) {
        marks.push({ end: data.end_ms, at: data.at + baseline + 0.2 });
      }
      if (data.type === 'overflow' || marks.length > 256) fail('Voice playback could not be verified.');
    };
    destination.stream.getTracks().forEach((track) => localSender.addTrack(track, destination.stream));
    // This is a local pair, so exchange complete SDP rather than also trickling
    // candidates already present in localDescription. One complete exchange
    // avoids candidate/description races and duplicate paths in WKWebView.
    atStage('loopback_offer');
    await localSender.setLocalDescription(await localSender.createOffer());
    assertAlive(id);
    atStage('loopback_offer_ice');
    await waitForICE(localSender, id);
    assertAlive(id);
    emit({ type: 'diagnostic', stage: 'loopback_offer_ice', name: 'ready', counts: candidateCounts(localSender.localDescription) });
    atStage('loopback_receiver_offer');
    await localReceiver.setRemoteDescription(localSender.localDescription);
    assertAlive(id);
    atStage('loopback_answer');
    await localReceiver.setLocalDescription(await localReceiver.createAnswer());
    assertAlive(id);
    atStage('loopback_answer_ice');
    await waitForICE(localReceiver, id);
    assertAlive(id);
    emit({ type: 'diagnostic', stage: 'loopback_answer_ice', name: 'ready', counts: candidateCounts(localReceiver.localDescription) });
    atStage('loopback_sender_answer');
    await localSender.setRemoteDescription(localReceiver.localDescription);
    assertAlive(id);
    // Billing may start only after the local receive path is actually connected.
    atStage('loopback_connected');
    await waitForConnected(localReceiver, id);
    assertAlive(id);
    emit({ type: 'diagnostic', stage: 'loopback_connected', name: 'ready' });
    return '';
  }
  async function dictation(id, processor) {
    atStage('audio_context');
    if (!window.AudioWorkletNode || !window.AudioContext) throw new Error('unsupported');
    const localContext = new AudioContext();
    context = localContext;
    await localContext.resume();
    assertAlive(id);
    if (!localContext.audioWorklet) throw new Error('unsupported');
    atStage('audio_worklet');
    const url = URL.createObjectURL(new Blob([processor], { type: 'text/javascript' }));
    try { await localContext.audioWorklet.addModule(url); }
    finally { URL.revokeObjectURL(url); }
    assertAlive(id);
    const localWorklet = new AudioWorkletNode(localContext, 'nyx-grok-audio', {
      numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1],
    });
    worklet = localWorklet;
    source = localContext.createMediaStreamSource(media);
    source.connect(localWorklet);
    // The processor outputs silence without a playback queue. Keep it running
    // for PCM24k capture without monitoring or recording the microphone.
    localWorklet.connect(localContext.destination);
    localWorklet.port.onmessage = ({ data }) => {
      if (!alive(id) || !capturing || data.generation !== captureGeneration || data.type !== 'pcm') return;
      if (!(data.bytes instanceof ArrayBuffer) || data.bytes.byteLength !== 960 || pendingPCM.size >= 100) {
        fail('Voice audio connection is too slow. Please reconnect.'); return;
      }
      const sequence = ++pcmSequence;
      pendingPCM.add(sequence);
      emit({ type: 'pcm', sequence, bytes: encode(data.bytes) });
    };
    return '';
  }
  async function prepare(protocol, id, processor) {
    cleanup();
    generation = id;
    closed = false;
    speakerMuted = false;
    captureGeneration = playbackGeneration = 0;
    pcmSequence = 0;
    atStage('runtime');
    emit({ type: 'diagnostic', stage: 'runtime', name: 'ready', counts: {
      secure_context: window.isSecureContext ? 1 : 0,
      opaque_document_origin: window.origin === 'null' ? 1 : 0,
      opaque_location_origin: window.location?.origin === 'null' ? 1 : 0,
      document_visible: document.hidden ? 0 : 1,
    } });
    if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia || !window.RTCPeerConnection) throw new Error('unsupported');
    audio = document.createElement('audio');
    audio.autoplay = true;
    audio.setAttribute('playsinline', '');
    document.body.appendChild(audio);
    atStage('microphone');
    const localMedia = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true }, video: false,
    });
    if (!alive(id)) { localMedia.getTracks().forEach((track) => track.stop()); throw new Error('cancelled'); }
    media = localMedia;
    localMedia.getAudioTracks().forEach((track) => { track.enabled = false; });
    if (protocol === 'dictation') return dictation(id, processor);
    if (protocol === 'xai_realtime') return grok(id, processor);
    if (protocol !== 'openai_live') throw new Error('unsupported');
    const localPeer = new RTCPeerConnection();
    peer = localPeer;
    localMedia.getTracks().forEach((track) => localPeer.addTrack(track, localMedia));
    localPeer.ontrack = ({ track, streams }) => {
      if (!alive(id)) return;
      audio.srcObject = streams[0] || new MediaStream([track]);
      void audio.play().catch(() => {});
    };
    localPeer.onconnectionstatechange = () => {
      if (alive(id) && localPeer.connectionState === 'failed') fail('WebRTC negotiation failed.');
    };
    const events = localPeer.createDataChannel('oai-events');
    events.onmessage = ({ data }) => {
      if (!alive(id) || typeof data !== 'string' || data.length > 262144) return;
      try { if (JSON.parse(data)?.type === 'session.started') emit({ type: 'provider-ready' }); }
      catch { /* Provider data never enters logs, storage, or the app API. */ }
    };
    await localPeer.setLocalDescription(await localPeer.createOffer());
    assertAlive(id);
    await waitForICE(localPeer, id);
    assertAlive(id);
    const sdp = localPeer.localDescription?.sdp;
    if (typeof sdp !== 'string' || !sdp || sdp.length > 131072) throw new Error('sdp');
    return sdp;
  }
  function input(enabled) {
    if (closed) return;
    media?.getAudioTracks().forEach((track) => { track.enabled = enabled; });
    if (capturing !== enabled) {
      capturing = enabled;
      worklet?.port.postMessage({ type: 'capture', enabled, generation: ++captureGeneration });
    }
  }
  async function command(message) {
    if (!message || !Number.isSafeInteger(message.generation)) return;
    const id = message.generation;
    if (message.type !== 'prepare' && !alive(id)) return;
    try {
      let result;
      switch (message.type) {
        case 'prepare': result = await prepare(message.protocol, id, window.__nyxVoiceWorklet); break;
        case 'answer':
          if (!peer || typeof message.sdp !== 'string' || message.sdp.length > 131072) throw new Error('sdp');
          await peer.setRemoteDescription({ type: 'answer', sdp: message.sdp }); break;
        case 'input': input(message.enabled === true); break;
        case 'hold': if (audio) audio.volume = message.pressed ? 0.25 : 1; break;
        case 'speaker': speakerMuted = message.muted === true; if (audio) audio.muted = speakerMuted; break;
        case 'resume': if (context) await context.resume(); assertAlive(id); if (audio) await audio.play(); break;
        case 'enqueue': {
          if (!worklet || !Number.isSafeInteger(message.end_ms) || message.end_ms < 0 || message.end_ms > 1830000) throw new Error('frame');
          const bytes = decode(message.bytes);
          worklet.port.postMessage({ type: 'audio', bytes, end_ms: message.end_ms, generation: playbackGeneration }, [bytes]);
          break;
        }
        case 'flush':
          ++playbackGeneration;
          worklet?.port.postMessage({ type: 'flush' }); marks = []; break;
        case 'pcm-ack': pendingPCM.delete(message.sequence); break;
        case 'close': cleanup(); break;
        default: return;
      }
      if (message.requestId && alive(id)) emit({ type: 'response', requestId: message.requestId, result });
    } catch (error) {
      if (!alive(id)) return;
      emit({ type: 'diagnostic', stage, name: typeof error?.name === 'string' ? error.name : 'Error' });
      const reason = error?.name === 'NotAllowedError' ? 'Microphone access was denied. Enable it in Settings to use voice.'
        : error?.name === 'NotFoundError' ? 'No microphone is available.'
        : error?.message === 'unsupported' ? 'Voice audio is not supported by this app runtime.'
        : error?.message === 'loopback' ? 'Voice playback could not be started. Try another voice source or continue with text.'
        : 'Voice audio could not be started. Please try again.';
      if (message.requestId) emit({ type: 'response', requestId: message.requestId, error: reason });
      fail(reason);
    }
  }
  window.__nyxVoiceMessage = (message) => { void command(message); };
  window.addEventListener('pagehide', cleanup);
  document.addEventListener('visibilitychange', () => { if (document.hidden && !closed) fail('Voice ended when the app left the foreground.'); });
  setInterval(() => { if (!closed) emit({ type: 'playback', ...playback() }); }, 250);
  post({ type: 'ready' });
})();
`;

// Same PCM resampling, bounded playback, and generation fences as the web client.
// Audio is transient; this processor neither records nor monitors the microphone.
export const VOICE_WORKLET_SOURCE = String.raw`/* global AudioWorkletProcessor, registerProcessor, sampleRate, currentTime */
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
`;

export function createVoiceMediaDocument(): string {
  const processor = JSON.stringify(VOICE_WORKLET_SOURCE).replace(
    /</g,
    "\\u003c",
  );
  return `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' blob:; connect-src 'none'; media-src blob:; style-src 'unsafe-inline'"></head><body><script>window.__nyxVoiceWorklet=${processor};${VOICE_MEDIA_ENGINE}</script></body></html>`;
}

const BASE64 =
  "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** Small, bounded PCM frames; no file or native recording API is involved. */
export function voicePcmToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  if (!bytes.length || bytes.length > 1920 || bytes.length % 2)
    throw new Error("Invalid voice audio frame");
  let result = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const value =
      (bytes[i]! << 16) | ((bytes[i + 1] ?? 0) << 8) | (bytes[i + 2] ?? 0);
    result +=
      BASE64[(value >>> 18) & 63]! +
      BASE64[(value >>> 12) & 63]! +
      (i + 1 < bytes.length ? BASE64[(value >>> 6) & 63]! : "=") +
      (i + 2 < bytes.length ? BASE64[value & 63]! : "=");
  }
  return result;
}

export function voicePcmFromBase64(value: string): ArrayBuffer {
  if (
    !value ||
    value.length > 2560 ||
    value.length % 4 ||
    !/^[A-Za-z0-9+/]+={0,2}$/.test(value)
  )
    throw new Error("Invalid voice audio frame");
  const size =
    (value.length / 4) * 3 -
    (value.endsWith("==") ? 2 : value.endsWith("=") ? 1 : 0);
  if (!size || size > 1920 || size % 2)
    throw new Error("Invalid voice audio frame");
  const bytes = new Uint8Array(size);
  let output = 0;
  for (let i = 0; i < value.length; i += 4) {
    const packed =
      (BASE64.indexOf(value[i]!) << 18) |
      (BASE64.indexOf(value[i + 1]!) << 12) |
      ((value[i + 2] === "=" ? 0 : BASE64.indexOf(value[i + 2]!)) << 6) |
      (value[i + 3] === "=" ? 0 : BASE64.indexOf(value[i + 3]!));
    if (output < size) bytes[output++] = (packed >>> 16) & 255;
    if (output < size) bytes[output++] = (packed >>> 8) & 255;
    if (output < size) bytes[output++] = packed & 255;
  }
  return bytes.buffer;
}

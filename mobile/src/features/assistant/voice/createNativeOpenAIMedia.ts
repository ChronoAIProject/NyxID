import { NativeModules, Platform } from "react-native";
import type { VoiceMediaAdapter } from "./AssistantVoiceClient";
import {
  NativeOpenAIMedia,
  type NativeAudioPeer,
  type NativeAudioStream,
} from "./NativeOpenAIMedia";
import { NativeVoiceRuntimeUnavailableError } from "./voiceStartError";

/** The native import stays outside the pure, injected media lifecycle. */
export function createNativeOpenAIMedia(): VoiceMediaAdapter | undefined {
  if (Platform.OS !== "ios" && Platform.OS !== "android") return undefined;
  if (!NativeModules.WebRTCModule) {
    throw new NativeVoiceRuntimeUnavailableError();
  }
  const { mediaDevices, RTCPeerConnection } =
    require("react-native-webrtc") as typeof import("react-native-webrtc");
  return new NativeOpenAIMedia({
    getUserMedia: async () =>
      (await mediaDevices.getUserMedia({
        audio: true,
        video: false,
      })) as unknown as NativeAudioStream,
    createPeer: () =>
      new RTCPeerConnection({ iceServers: [] }) as unknown as NativeAudioPeer,
  });
}

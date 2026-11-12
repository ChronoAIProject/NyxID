import { Platform } from "react-native";
import type { VoiceMediaAdapter } from "./AssistantVoiceClient";
import {
  NativeOpenAIMedia,
  type NativeAudioPeer,
  type NativeAudioStream,
} from "./NativeOpenAIMedia";

/** The native import stays outside the pure, injected media lifecycle. */
export function createNativeOpenAIMedia(): VoiceMediaAdapter | undefined {
  // The iOS path is validated with the universal native app. Keep Android on
  // its existing media engine until native Android routing is verified.
  if (Platform.OS !== "ios") return undefined;
  // Keep the native module out of Android/web startup and older Android clients.
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

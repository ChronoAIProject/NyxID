import Constants from "expo-constants";
import { NativeModules, Platform } from "react-native";
import type { VoiceMediaAdapter } from "./AssistantVoiceClient";
import {
  NativeOpenAIMedia,
  type NativeAudioPeer,
  type NativeAudioStream,
} from "./NativeOpenAIMedia";

/** The native import stays outside the pure, injected media lifecycle. */
export function createNativeOpenAIMedia(): VoiceMediaAdapter | undefined {
  if (Platform.OS === "android") {
    // Older Android binaries can lack the native module. Select WebView
    // before importing the package or acquiring the microphone.
    if (
      Constants.expoConfig?.extra?.androidOpenAIVoiceEngine !== "native" ||
      !NativeModules.WebRTCModule
    ) {
      return undefined;
    }
  } else if (Platform.OS !== "ios") {
    return undefined;
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

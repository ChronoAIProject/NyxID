import { requireOptionalNativeModule } from "expo-modules-core";

export type StartupKind = "cold" | "resume";
export type PerformanceScreen = "nyxbot_home" | "chat" | "auth";
export type FrameScenario =
  | "chat_drag"
  | "chat_drag_during_stream"
  | "chat_streaming";
export type StartupTiming = {
  duration_ms: number;
  startup_source:
    | "android_process_uptime"
    | "ios_did_finish_launching"
    | "android_activity_resume"
    | "ios_will_enter_foreground";
};
export type FrameTiming = {
  measurement_source: "android_frame_metrics" | "ios_display_link";
  duration_ms: number;
  sample_count: number;
  interval_p50_ms: number;
  interval_p95_ms: number;
  interval_max_ms: number;
  over_budget_count: number;
  dropped_callback_count?: number;
};
export interface NativePerformance {
  measureReady(kind: StartupKind): Promise<StartupTiming | null>;
  startFrames(token: string): Promise<boolean>;
  stopFrames(token: string): Promise<FrameTiming | null>;
  cancelFrames(): Promise<void>;
}

/** Missing native binary (Expo Go/older install) disables these metrics. */
export const nativePerformance =
  requireOptionalNativeModule<NativePerformance>("NyxPerformance");

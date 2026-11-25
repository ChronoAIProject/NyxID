import type { LaunchBridge } from "./launchRuntime";
import { requireOptionalNativeModule } from "expo-modules-core";

export type FrameScenario =
  | "chat_drag"
  | "chat_drag_during_stream"
  | "chat_streaming";
export type FrameTiming = {
  measurement_source: "android_frame_metrics" | "ios_display_link";
  duration_ms: number;
  sample_count: number;
  interval_p50_ms: number;
  interval_p95_ms: number;
  interval_max_ms: number;
  over_budget_count: number;
  dropped_callback_count?: number;
  observed_fps?: number;
  peak_rss_bytes?: number;
  peak_rss_source?: "android_proc_status_vmhwm" | "ios_getrusage_maxrss";
};
export interface NativePerformance extends LaunchBridge {
  startFrames(token: string): Promise<boolean>;
  stopFrames(token: string): Promise<FrameTiming | null>;
  cancelFrames(): Promise<void>;
}

/** Missing native binary (Expo Go/older install) disables these metrics. */
export const nativePerformance =
  requireOptionalNativeModule<NativePerformance>("NyxPerformance");

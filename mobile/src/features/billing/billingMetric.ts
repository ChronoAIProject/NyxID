import { translateApp } from "../../i18n/appTranslation";

// Display names only. The server's stable metric IDs remain unchanged for
// billing, grouping and requests; unknown future metrics remain identifiable.
const METRIC_NAMES: Record<string, string> = {
  tokens: "tokens",
  requests: "requests",
  bytes: "bytes",
  input_tokens: "input tokens",
  output_tokens: "output tokens",
  cache_read_tokens: "cache read tokens",
  cache_write_tokens: "cache write tokens",
  reasoning_tokens: "reasoning tokens",
  image_input_tokens: "image input tokens",
  image_output_tokens: "image output tokens",
  audio_input_tokens: "audio input tokens",
  audio_output_tokens: "audio output tokens",
  images: "images",
  voice_seconds: "voice seconds",
  machine_running_seconds: "machine running seconds",
  sandbox_cpu_millis_seconds: "sandbox CPU millicore-seconds",
  sandbox_memory_mib_seconds: "sandbox memory MiB-seconds",
  sandbox_disk_gib_seconds: "sandbox disk GiB-seconds",
};

export function billingMetricName(metric: string): string {
  return Object.hasOwn(METRIC_NAMES, metric)
    ? translateApp(METRIC_NAMES[metric]!)
    : metric.replaceAll("_", " ");
}

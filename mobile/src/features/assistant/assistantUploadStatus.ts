import { translateApp } from "../../i18n/appTranslation";
import type { PendingAssistantUpload } from "./AssistantUploadSession";

export type UploadStatus = {
  text?: string;
  loader: "none" | "determinate" | "indeterminate";
  progress: number;
};

/** Completion and errors take precedence over stale transport progress. */
export function assistantUploadStatus(
  row: Pick<PendingAssistantUpload, "item" | "error" | "progress" | "resumable">,
): UploadStatus {
  if (row.error) return { text: row.error, loader: "none", progress: 0 };
  if (row.resumable)
    return { text: translateApp("Ready to resume"), loader: "none", progress: 0 };
  if (row.item) return { loader: "none", progress: 100 };
  const progress = Number.isFinite(row.progress)
    ? Math.max(0, Math.min(100, row.progress))
    : 0;
  if (progress >= 99)
    return { text: translateApp("Checking file…"), loader: "indeterminate", progress };
  if (progress > 0)
    return {
      text: translateApp("Uploading {{percent}}%", {
        percent: Math.round(progress),
      }),
      loader: "determinate",
      progress,
    };
  return { text: translateApp("Waiting to upload…"), loader: "indeterminate", progress: 0 };
}

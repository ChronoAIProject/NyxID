export const CHAT_STREAM_BATCH_INTERVAL_MS = 50;
export const CHAT_STREAM_MAX_BATCH_FRAMES = 256;
export const CHAT_STREAM_MAX_ERROR_BODY_CHARS = 65_536;

export type ChatStreamFrame = Record<string, unknown>;

export type ChatStreamWorkerCommand =
  | {
      readonly type: "stream.start";
      readonly requestId: string;
      readonly url: string;
      readonly bodyText: string;
    }
  | {
      readonly type: "stream.cancel";
      readonly requestId: string;
    };

export type ChatStreamWorkerMessage =
  | {
      readonly type: "stream.response";
      readonly requestId: string;
      readonly status: number;
      readonly contentType: string;
    }
  | {
      readonly type: "stream.batch";
      readonly requestId: string;
      readonly frames: readonly ChatStreamFrame[];
    }
  | {
      readonly type: "stream.complete";
      readonly requestId: string;
      readonly frames: readonly ChatStreamFrame[];
    }
  | {
      readonly type: "stream.http_error";
      readonly requestId: string;
      readonly status: number;
      readonly body: string;
    }
  | {
      readonly type: "stream.network_error";
      readonly requestId: string;
      readonly code: "network_error" | "stream_closed" | "worker_error";
      readonly message: string;
    }
  | {
      readonly type: "stream.cancelled";
      readonly requestId: string;
    };

export function isTerminalChatStreamFrame(frame: ChatStreamFrame): boolean {
  const explicitType = frame["type"];
  if (typeof explicitType === "string") {
    const normalized = explicitType.toUpperCase();
    if (
      normalized === "RUN_FINISHED" ||
      normalized === "RUN_ERROR" ||
      normalized === "RUN_STOPPED"
    ) {
      return true;
    }
  }
  return Boolean(
    frame["runFinished"] ?? frame["runError"] ?? frame["runStopped"],
  );
}

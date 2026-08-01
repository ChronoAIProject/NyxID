export const CHAT_STREAM_BATCH_INTERVAL_MS = 50;
export const CHAT_STREAM_MAX_BATCH_FRAMES = 256;
export const CHAT_STREAM_MAX_ERROR_BODY_CHARS = 65_536;
export const CHAT_STREAM_MAX_BODY_BYTES = 64 * 1024;
export const CHAT_STREAM_MAX_WIRE_BYTES = 512 * 1024;
export const CHAT_STREAM_MAX_WIRE_BATCH_BYTES = 32 * 1024;

export type ChatStreamFrame = Record<string, unknown>;

export type ChatStreamWireOutcome =
  | "complete"
  | "cancelled"
  | "network_error"
  | "worker_error"
  | "protocol_cancel";

export interface ChatStreamWireLineFragment {
  readonly text: string;
  readonly ending?: "\n" | "\r\n" | "\r" | "";
  readonly fragment: boolean;
}

export type ChatStreamWireEvent =
  | {
      readonly type: "lines";
      readonly requestId: string;
      readonly lines: readonly {
        readonly text: string;
        readonly ending: "\n" | "\r\n" | "\r" | "";
      }[];
      readonly bytes: number;
      readonly truncated: boolean;
    }
  | {
      readonly type: "body";
      readonly requestId: string;
      readonly text: string;
      readonly bytes: number;
      readonly truncated: boolean;
    }
  | {
      readonly type: "end";
      readonly requestId: string;
      readonly outcome: ChatStreamWireOutcome;
    };

export type ChatStreamWorkerCommand =
  | {
      readonly type: "stream.start";
      readonly requestId: string;
      readonly url: string;
      readonly bodyText: string;
      readonly headers?: Readonly<Record<string, string>>;
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
      readonly debugUpstream?: string;
    }
  | {
      readonly type: "stream.batch";
      readonly requestId: string;
      readonly frames: readonly ChatStreamFrame[];
    }
  | {
      readonly type: "stream.wire_batch";
      readonly requestId: string;
      readonly fragments: readonly ChatStreamWireLineFragment[];
      readonly bytes: number;
      readonly truncated: boolean;
    }
  | {
      readonly type: "stream.wire_body";
      readonly requestId: string;
      readonly text: string;
      readonly bytes: number;
      readonly truncated: boolean;
    }
  | {
      readonly type: "stream.wire_end";
      readonly requestId: string;
      readonly outcome: ChatStreamWireOutcome;
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
      readonly debugUpstream?: string;
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

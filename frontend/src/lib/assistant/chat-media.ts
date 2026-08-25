import { AGUIEventType, parseCustomEvent, type AGUIEvent } from "./agui-types";
import type { ArtifactContentBlock } from "@/types/assistant";

type JsonRecord = Record<string, unknown>;

export const MAX_MEDIA_DATA_CHARS = 8_000_000;

export interface ChatMediaContent {
  readonly dataBase64?: string;
  readonly mediaType: string;
  readonly name: string;
  readonly preview: string | null;
  readonly uri?: string;
}

export type ChatMediaPresentation =
  | { readonly artifact: ArtifactContentBlock; readonly notice?: never }
  | { readonly artifact?: never; readonly notice: string };

function asRecord(value: unknown): JsonRecord | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as JsonRecord)
    : null;
}

function unpackMediaPayload(value: unknown): JsonRecord | null {
  const record = asRecord(value);
  if (!record) return null;
  const nested = asRecord(record.value) ?? record;
  return asRecord(nested.part) ?? nested;
}

function stringField(record: JsonRecord, ...keys: string[]): string {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string") return value;
  }
  return "";
}

export function extractMediaContent(event: AGUIEvent): ChatMediaContent | null {
  let payload: JsonRecord | null = null;
  if (event.type === AGUIEventType.MEDIA_CONTENT) {
    payload = unpackMediaPayload(event);
  } else if (event.type === AGUIEventType.CUSTOM) {
    const custom = parseCustomEvent(event);
    if (custom.name !== "MEDIA_CONTENT") return null;
    payload = unpackMediaPayload(custom.data);
  }
  if (!payload) return null;

  const dataBase64 = stringField(payload, "dataBase64", "data").trim();
  const uri = stringField(payload, "uri", "url").trim();
  const preview = stringField(payload, "text").trim();
  if (!dataBase64 && !uri && !preview) return null;
  return {
    ...(dataBase64 ? { dataBase64 } : {}),
    mediaType:
      stringField(payload, "mediaType", "mimeType").trim() ||
      "application/octet-stream",
    name: stringField(payload, "name").trim() || "attachment",
    preview: preview || null,
    ...(uri ? { uri } : {}),
  };
}

function safeMediaUrl(value: string): string | undefined {
  if (!value) return undefined;
  try {
    const parsed = new URL(
      value,
      typeof globalThis.location?.origin === "string"
        ? globalThis.location.origin
        : "http://localhost",
    );
    return parsed.protocol === "https:" || parsed.protocol === "http:"
      ? parsed.toString()
      : undefined;
  } catch {
    return undefined;
  }
}

function base64SizeBytes(value: string): number {
  return Math.floor((value.length * 3) / 4);
}

export function presentMediaContent(
  media: ChatMediaContent,
  index: number,
): ChatMediaPresentation {
  const artifactBase = {
    type: "artifact" as const,
    block_id: `runtime-artifact:${String(index)}`,
    artifact_id: `runtime-media:${String(index)}`,
    name: media.name,
    mime: media.mediaType,
    preview: media.preview,
  };
  if (
    media.dataBase64 &&
    media.dataBase64.length <= MAX_MEDIA_DATA_CHARS
  ) {
    return {
      artifact: {
        ...artifactBase,
        size_bytes: base64SizeBytes(media.dataBase64),
        download_url: `data:${media.mediaType};base64,${media.dataBase64}`,
      },
    };
  }
  const downloadUrl = safeMediaUrl(media.uri ?? "");
  if (downloadUrl) {
    return {
      artifact: {
        ...artifactBase,
        size_bytes: 0,
        download_url: downloadUrl,
      },
    };
  }
  return {
    notice: `The assistant produced an attachment (${media.name}) that is too large to display here.`,
  };
}

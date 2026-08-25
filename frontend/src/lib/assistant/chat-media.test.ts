import { describe, expect, it } from "vitest";
import { AGUIEventType } from "./agui-types";
import {
  extractMediaContent,
  MAX_MEDIA_DATA_CHARS,
  presentMediaContent,
} from "./chat-media";

describe("chat media presentation", () => {
  it("embeds media at the inline cap as a data URL artifact", () => {
    const media = extractMediaContent({
      type: AGUIEventType.MEDIA_CONTENT,
      dataBase64: "aGVsbG8=",
      mediaType: "image/png",
      name: "chart.png",
    });
    expect(presentMediaContent(media!, 1)).toMatchObject({
      artifact: {
        name: "chart.png",
        mime: "image/png",
        size_bytes: 6,
        download_url: "data:image/png;base64,aGVsbG8=",
      },
    });
  });

  it("refuses to embed base64 content above 8 million characters", () => {
    const oversized = presentMediaContent(
      {
        dataBase64: "A".repeat(MAX_MEDIA_DATA_CHARS + 1),
        mediaType: "application/octet-stream",
        name: "oversized.bin",
        preview: null,
      },
      1,
    );
    expect(oversized).toEqual({
      notice:
        "The assistant produced an attachment (oversized.bin) that is too large to display here.",
    });
  });

  it("accepts completion-builder CUSTOM media frames", () => {
    expect(
      extractMediaContent({
        type: AGUIEventType.CUSTOM,
        name: "MEDIA_CONTENT",
        value: {
          sessionId: "message-1",
          part: {
            dataBase64: "dGV4dA==",
            mediaType: "text/plain",
            name: "notes.txt",
          },
        },
      }),
    ).toMatchObject({
      dataBase64: "dGV4dA==",
      mediaType: "text/plain",
      name: "notes.txt",
    });
  });
});

import { useEffect, useState } from "react";
import { ImageOff } from "lucide-react";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import type { ChatImage } from "@/lib/assistant/chat-types";

/**
 * An image a tool returned during a turn. It is fetched through the
 * authenticated assistant client and shown from a local object URL, so the
 * owner-only attachment route never needs cookies or a public link.
 */
export function ToolImage({ image }: { readonly image: ChatImage }) {
  const [url, setUrl] = useState<string>();
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    // Callers key each image by its attachment id, which maps to exactly one
    // endpoint, so state never needs resetting for a different image.
    let active = true;
    let objectUrl: string | undefined;
    assistantHttp(image.endpoint)
      .then((response) => response.blob())
      .then((blob) => {
        if (!active) return;
        objectUrl = URL.createObjectURL(blob);
        setUrl(objectUrl);
      })
      .catch(() => {
        if (active) setFailed(true);
      });
    return () => {
      active = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [image.endpoint]);

  const alt = `Image from ${image.label}`;
  if (failed) {
    return (
      <div
        role="status"
        className="flex items-center gap-2 rounded-lg border border-hairline px-3 py-2 text-[11px] text-muted-foreground"
      >
        <ImageOff className="h-3.5 w-3.5" aria-hidden="true" />
        Image unavailable
      </div>
    );
  }
  if (!url) {
    return (
      <div
        role="status"
        aria-label={`Loading ${alt.toLowerCase()}`}
        className="h-40 w-64 animate-pulse rounded-lg border border-hairline bg-overlay/35"
      />
    );
  }
  return (
    <a href={url} target="_blank" rel="noopener noreferrer" className="block">
      <img
        src={url}
        alt={alt}
        className="max-h-80 max-w-full rounded-lg border border-hairline object-contain"
      />
    </a>
  );
}

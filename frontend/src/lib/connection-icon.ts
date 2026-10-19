import type { KeyInfo } from "@/types/keys";

/**
 * The image a connection displays: its own icon override, else the catalog
 * service's icon (an uploaded provider or Tool icon). `null` lets
 * `ServiceIcon` fall back to the slug glyph.
 */
export function connectionIconUrl(
  key: Pick<KeyInfo, "icon_url" | "catalog_icon_url"> | null | undefined,
): string | null {
  return key?.icon_url || key?.catalog_icon_url || null;
}

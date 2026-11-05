import type { ServiceIconProps } from "./index";
import { XaiGlyph } from "./_shared";

export default function LlmXaiIcon({ size = 20, color }: ServiceIconProps) {
  return <XaiGlyph data-slug="llm-xai" size={size} color={color} />;
}

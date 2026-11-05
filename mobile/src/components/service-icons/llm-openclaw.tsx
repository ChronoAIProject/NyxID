// OpenClaw catalog tile. Hand-built 3-talon claw silhouette (lives in
// `_shared.tsx`); reads literally as the product's name. No Simple Icons
// entry — OpenClaw has no public canonical brand mark.
import { OpenClawGlyph } from "./_shared";

export default function LlmOpenClawIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <OpenClawGlyph data-slug="llm-openclaw" size={size} color={color} />;
}

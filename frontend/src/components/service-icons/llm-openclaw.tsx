// OpenClaw catalog tile. Hand-built 3-talon claw silhouette (lives in
// `_shared.tsx`); reads literally as the product's name. No Simple Icons
// entry — OpenClaw has no public canonical brand mark.
import { OpenClawGlyph } from "./_shared";

export default function LlmOpenClawIcon({
  className,
}: {
  className?: string;
}) {
  return <OpenClawGlyph data-slug="llm-openclaw" className={className} />;
}

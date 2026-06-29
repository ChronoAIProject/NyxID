// Pure brand glyph for the OpenClaw catalog tile. Hand-built 4-toe + 1-pad
// "claw" silhouette (single-tone only).
import { OpenClawGlyph } from "./_shared";

export default function LlmOpenClawIcon({
  className,
}: {
  className?: string;
}) {
  return <OpenClawGlyph data-slug="llm-openclaw" className={className} />;
}

// Pure brand glyph for the ElevenLabs catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { ElevenLabsGlyph } from "./_shared";

export default function ApiElevenLabsIcon({
  className,
}: {
  className?: string;
}) {
  return <ElevenLabsGlyph data-slug="api-elevenlabs" className={className} />;
}

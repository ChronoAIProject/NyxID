// Pure brand glyph for the ElevenLabs catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { ElevenLabsGlyph } from "./_shared";

export default function ApiElevenLabsIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return (
    <ElevenLabsGlyph data-slug="api-elevenlabs" size={size} color={color} />
  );
}

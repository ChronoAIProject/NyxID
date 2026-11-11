// Pure brand glyph for the Spotify catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { SpotifyGlyph } from "./_shared";

export default function ApiSpotifyIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <SpotifyGlyph data-slug="api-spotify" size={size} color={color} />;
}

// Pure brand glyph for the Spotify catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { SpotifyGlyph } from "./_shared";

export default function ApiSpotifyIcon({
  className,
}: {
  className?: string;
}) {
  return <SpotifyGlyph data-slug="api-spotify" className={className} />;
}

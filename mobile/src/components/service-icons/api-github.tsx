// Pure brand glyph for the GitHub catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { GithubGlyph } from "./_shared";

export default function ApiGithubIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return <GithubGlyph data-slug="api-github" size={size} color={color} />;
}

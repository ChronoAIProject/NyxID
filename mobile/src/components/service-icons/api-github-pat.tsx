// GitHub PAT catalog tile: GitHub Octocat + Lucide `KeyRound` badge (the
// "key" hint differentiates the API-key variant from the plain OAuth GitHub
// service `api-github.tsx`). See `CompositeBadgeWrapper` in `_shared.tsx` for
// the visual layering rules.
import { KeyRound } from "lucide-react-native";
import { CompositeBadgeWrapper, GithubGlyph } from "./_shared";

export default function ApiGithubPatIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return (
    <CompositeBadgeWrapper
      size={size}
      color={color}
      badge={<KeyRound strokeWidth={2.5} size={size * 0.7 - 2} color={color} />}
    >
      <GithubGlyph data-slug="api-github-pat" size={size} color={color} />
    </CompositeBadgeWrapper>
  );
}

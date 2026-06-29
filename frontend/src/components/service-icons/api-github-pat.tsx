// GitHub PAT catalog tile: GitHub Octocat + Lucide `KeyRound` badge (the
// "key" hint differentiates the API-key variant from the plain OAuth GitHub
// service `api-github.tsx`). See `CompositeBadgeWrapper` in `_shared.tsx` for
// the visual layering rules.
import { KeyRound } from "lucide-react";
import { CompositeBadgeWrapper, GithubGlyph } from "./_shared";

export default function ApiGithubPatIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<KeyRound className="h-3.5 w-3.5" strokeWidth={2.5} />}
    >
      <GithubGlyph data-slug="api-github-pat" className="h-5 w-5" />
    </CompositeBadgeWrapper>
  );
}

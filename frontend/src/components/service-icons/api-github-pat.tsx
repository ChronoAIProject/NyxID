// Composite glyph for the GitHub (PAT) catalog tile: GitHub Octocat + Lucide
// `KeyRound` badge in NyxID accent purple. The accent (second tone) lives on
// the Lucide badge wrapper; the primary brand glyph stays `currentColor`
// only.
import { KeyRound } from "lucide-react";
import { GithubGlyph } from "./_shared";

export default function ApiGithubPatIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <span
      className={`relative inline-flex h-5 w-5 items-center justify-center ${
        className ?? ""
      }`}
    >
      <GithubGlyph data-slug="api-github-pat" className="h-5 w-5" />
      <KeyRound
        aria-hidden="true"
        className="absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 text-nyx-secondary-400 bg-card/95 rounded-sm p-px"
      />
    </span>
  );
}

// Composite glyph for the OpenAI Codex catalog tile: OpenAI knot + Lucide
// `Code` badge in NyxID accent purple. The accent (second tone) lives on the
// Lucide badge wrapper; the primary brand glyph stays `currentColor` only.
import { Code } from "lucide-react";
import { OpenAiGlyph } from "./_shared";

export default function LlmOpenaiCodexIcon({
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
      <OpenAiGlyph data-slug="llm-openai-codex" className="h-5 w-5" />
      <Code
        aria-hidden="true"
        className="absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 text-nyx-secondary-400 bg-card/95 rounded-sm p-px"
      />
    </span>
  );
}

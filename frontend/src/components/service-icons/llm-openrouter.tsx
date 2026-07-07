// OpenRouter catalog tile. Official OpenRouter mark (route lines fanning
// into arrowheads) taken from openrouter.ai's own logo markup, flattened
// to currentColor to match the rest of the tile grid.
import { OpenRouterGlyph } from "./_shared";

export default function LlmOpenRouterIcon({
  className,
}: {
  className?: string;
}) {
  return <OpenRouterGlyph data-slug="llm-openrouter" className={className} />;
}

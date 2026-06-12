/**
 * Parse a free-form "additional scopes" textbox into a trimmed, deduped list.
 * Accepts comma-, space-, or newline-separated values. Mirrors the CLI's
 * `--scope` flag and the backend's `parse_additional_scopes` splitter so that
 * input is forgiving regardless of how the user pastes scopes from docs.
 *
 * Shared by the dashboard's add-key dialog and the CLI pair wizard
 * (NyxID#917) so both surfaces stay in lockstep with the backend splitter.
 */
export function parseAdditionalScopes(raw: string): readonly string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const piece of raw.split(/[,\s]+/)) {
    const trimmed = piece.trim();
    if (trimmed && !seen.has(trimmed)) {
      seen.add(trimmed);
      out.push(trimmed);
    }
  }
  return out;
}

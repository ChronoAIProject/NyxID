/**
 * Composers show one row while unfocused with nothing but whitespace.
 * Dictation can keep its active layout; attachments retain their own strip.
 * The draft itself is never trimmed: only the layout reads the trimmed text.
 */
export function composerCompact(
  compactWhenIdle: boolean,
  focused: boolean,
  value: string,
) {
  return compactWhenIdle && !focused && value.trim().length === 0;
}

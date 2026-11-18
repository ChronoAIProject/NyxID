/**
 * Phone composers show one row only while unfocused with nothing but
 * whitespace; the caller's `compactWhenIdle` already excludes attachments,
 * uploads and dictation. The draft itself is never trimmed: only the layout
 * reads the trimmed text.
 */
export function composerCompact(
  compactWhenIdle: boolean,
  focused: boolean,
  value: string,
) {
  return compactWhenIdle && !focused && value.trim().length === 0;
}

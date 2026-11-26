/** Remove edge whitespace/format marks and normalize scheme case/punctuation.
 * Interior characters stay intact; the schema preserves the original URL rules.
 */
export function normalizeUrlInput(value: string): string {
  return value
    .replace(/^[\p{Cf}\s]+|[\p{Cf}\s]+$/gu, "")
    .replace(
      /^([a-z][a-z0-9+.-]*)([:：])([/／])([/／])/i,
      (_separator, scheme: string) => `${scheme.toLowerCase()}://`,
    );
}

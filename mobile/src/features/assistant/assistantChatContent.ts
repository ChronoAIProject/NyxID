const PIPE = String.raw`[\|\uff5c]`;
const DSML_FUNCTION_CALLS_BLOCK_PATTERN =
  String.raw`<\s*` +
  PIPE +
  String.raw`\s*DSML\s*` +
  PIPE +
  String.raw`\s*function_calls\s*>[\s\S]*?<\/\s*` +
  PIPE +
  String.raw`\s*DSML\s*` +
  PIPE +
  String.raw`\s*function_calls\s*>`;
const DSML_FUNCTION_CALLS_OPEN_PATTERN =
  String.raw`<\s*` +
  PIPE +
  String.raw`\s*DSML\s*` +
  PIPE +
  String.raw`\s*function_calls\s*>`;
const DSML_FUNCTION_CALLS_CLOSE_PATTERN =
  String.raw`<\/\s*` +
  PIPE +
  String.raw`\s*DSML\s*` +
  PIPE +
  String.raw`\s*function_calls\s*>`;

const XML_FUNCTION_CALLS_BLOCK_PATTERN = String.raw`<function_calls\s*>[\s\S]*?<\/function_calls\s*>`;
const XML_FUNCTION_CALLS_OPEN_PATTERN = String.raw`<function_calls\s*>`;
const XML_FUNCTION_CALLS_CLOSE_PATTERN = String.raw`<\/function_calls\s*>`;

const FUNCTION_CALL_PATTERNS: readonly (readonly [string, string, string])[] = [
  [
    DSML_FUNCTION_CALLS_BLOCK_PATTERN,
    DSML_FUNCTION_CALLS_OPEN_PATTERN,
    DSML_FUNCTION_CALLS_CLOSE_PATTERN,
  ],
  [
    XML_FUNCTION_CALLS_BLOCK_PATTERN,
    XML_FUNCTION_CALLS_OPEN_PATTERN,
    XML_FUNCTION_CALLS_CLOSE_PATTERN,
  ],
];

export function sanitizeAssistantMessageContent(content: string): string {
  if (!content) return "";

  let sanitized = content;
  for (const [blockPattern] of FUNCTION_CALL_PATTERNS) {
    sanitized = sanitized.replace(new RegExp(blockPattern, "gi"), "\n");
  }

  const danglingBlockStart = findDanglingFunctionCallBlockStart(sanitized);
  if (danglingBlockStart >= 0) {
    sanitized = sanitized.slice(0, danglingBlockStart);
  }

  return sanitized
    .replace(/\n[ \t]+\n/g, "\n\n")
    .replace(/\n{3,}/g, "\n\n")
    .trimEnd();
}

function findDanglingFunctionCallBlockStart(content: string): number {
  let earliest = -1;
  for (const [, openPattern, closePattern] of FUNCTION_CALL_PATTERNS) {
    const matchIndex = findDanglingStart(content, openPattern, closePattern);
    if (matchIndex >= 0 && (earliest < 0 || matchIndex < earliest)) {
      earliest = matchIndex;
    }
  }
  return earliest;
}

function findDanglingStart(
  content: string,
  openPatternSource: string,
  closePatternSource: string,
): number {
  let searchIndex = 0;
  while (searchIndex < content.length) {
    const openPattern = new RegExp(openPatternSource, "gi");
    openPattern.lastIndex = searchIndex;
    const openMatch = openPattern.exec(content);
    if (!openMatch) return -1;

    const closePattern = new RegExp(closePatternSource, "gi");
    closePattern.lastIndex = openMatch.index + openMatch[0].length;
    const closeMatch = closePattern.exec(content);
    if (!closeMatch) return openMatch.index;
    searchIndex = closeMatch.index + closeMatch[0].length;
  }
  return -1;
}

/** Assistant-upload display names mirror attachment_filename::sanitize_name. */
export const ASSISTANT_FILENAME_CODEPOINTS = 120;
export const ASSISTANT_FILENAME_HEADER_BYTES = 1024;

const UNSAFE = /[^\p{Alphabetic}\p{N}._ ()-]/gu;
const ALPHANUMERIC = /^[\p{Alphabetic}\p{N}]+$/u;
const leading = (name: string) => name.replace(/^[ .]+/u, "");
const trimmed = (name: string) => name.replace(/^[ .]+|[ .]+$/gu, "");

/** Undefined means there is no usable original; callers must not invent one. */
export function sanitizeAssistantUploadName(input: string): string | undefined {
  // Trim the full trailing run before suffix/stem selection, including padding
  // exposed by characters that sanitization removes (for example a final emoji).
  const leaf = (input.split(/[/\\]/).pop() ?? "").replace(
    /[^\p{Alphabetic}\p{N}_()-]+$/u,
    "",
  );
  const dot = leaf.lastIndexOf(".");
  const rawStem = dot < 0 ? leaf : leaf.slice(0, dot);
  if (dot >= 0 && rawStem && !rawStem.replace(UNSAFE, "").replace(/[ .]/g, ""))
    return undefined;

  const prefix = [...leading(leaf.replace(UNSAFE, ""))].slice(
    0,
    ASSISTANT_FILENAME_CODEPOINTS + 1,
  );
  let name = prefix.slice(0, ASSISTANT_FILENAME_CODEPOINTS).join("");
  if (prefix.length > ASSISTANT_FILENAME_CODEPOINTS && dot >= 0) {
    const suffix = [...leaf.slice(dot + 1).replace(UNSAFE, "")]
      .slice(0, ASSISTANT_FILENAME_CODEPOINTS + 1)
      .join("");
    const suffixLength = [...suffix].length;
    if (
      ALPHANUMERIC.test(suffix) &&
      suffixLength + 2 <= ASSISTANT_FILENAME_CODEPOINTS
    ) {
      const stem = [...leading(rawStem.replace(UNSAFE, ""))]
        .slice(0, ASSISTANT_FILENAME_CODEPOINTS - suffixLength - 1)
        .join("")
        .trimEnd();
      name = `${stem}.${suffix}`;
    }
  }
  return trimmed(name) || undefined;
}

export function assistantUploadExtension(input: string): string {
  const name = sanitizeAssistantUploadName(input) ?? "attachment";
  const dot = name.lastIndexOf(".");
  return dot < 0 ? "" : name.slice(dot + 1).toLowerCase();
}

/** The encoded ASCII header itself must fit the server's 1024-byte cap. */
export function assistantUploadNameHeader(input: string): string | undefined {
  const name = sanitizeAssistantUploadName(input);
  if (!name) return undefined;
  if (encodeURIComponent(name).length <= ASSISTANT_FILENAME_HEADER_BYTES)
    return encodeURIComponent(name);

  const dot = name.lastIndexOf(".");
  const suffix =
    dot > 0 && ALPHANUMERIC.test(name.slice(dot + 1)) ? name.slice(dot) : "";
  const stem = suffix ? name.slice(0, dot) : name;
  let kept = "";
  for (const character of stem) {
    const next = kept + character;
    if (
      encodeURIComponent(next + suffix).length > ASSISTANT_FILENAME_HEADER_BYTES
    )
      break;
    kept = next;
  }
  kept = trimmed(kept);
  // An oversized suffix must not become a different dialect or a fake original.
  return kept ? encodeURIComponent(kept + suffix) : undefined;
}

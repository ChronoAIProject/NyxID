/** Bounded RFC 4180-style CSV, including quoted newlines and doubled quotes. */
export function parseCrmImport(source: string): Record<string, unknown>[] {
  if (new TextEncoder().encode(source).length > 512_000)
    throw new Error("Use a file of at most 512 KB.");
  if (/^(?:\[|{)/.test(source.trimStart())) {
    const document: unknown = JSON.parse(source);
    const parsed: unknown =
      document && typeof document === "object" && "records" in document
        ? document.records
        : document;
    if (
      !Array.isArray(parsed) ||
      parsed.some(
        (row) => !row || typeof row !== "object" || Array.isArray(row),
      )
    )
      throw new Error("JSON must be an array of row objects.");
    if (parsed.length < 1 || parsed.length > 100)
      throw new Error("Import 1–100 rows per batch.");
    if (new Set(parsed.flatMap((row) => Object.keys(row))).size > 100)
      throw new Error("Use at most 100 columns across all rows.");
    return parsed as Record<string, unknown>[];
  }
  const lines: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let quoted = false;
  let endedQuote = false;
  for (let i = 0; i < source.length; i++) {
    const char = source[i];
    if (quoted) {
      if (char === '"') {
        if (source[i + 1] === '"') {
          cell += '"';
          i++;
        } else {
          quoted = false;
          endedQuote = true;
        }
      } else cell += char;
    } else if (char === '"') {
      if (cell || endedQuote) throw new Error("Malformed CSV quote.");
      quoted = true;
    } else if (char === "," || char === "\n" || char === "\r") {
      row.push(cell);
      cell = "";
      endedQuote = false;
      if (char !== ",") {
        if (char === "\r" && source[i + 1] === "\n") i++;
        lines.push(row);
        row = [];
        if (lines.length > 102) throw new Error("Import at most 100 rows.");
      }
    } else {
      if (endedQuote) throw new Error("Unexpected text after CSV quote.");
      cell += char;
    }
  }
  if (quoted) throw new Error("Unclosed CSV quote.");
  if (cell || row.length || endedQuote) {
    row.push(cell);
    lines.push(row);
  }
  const headers =
    lines.shift()?.map((value) => value.replace(/^\uFEFF/, "")) ?? [];
  if (
    !headers.length ||
    headers.length > 100 ||
    headers.some((h) => !h.trim()) ||
    new Set(headers).size !== headers.length
  )
    throw new Error("Use 1–100 unique, nonblank column names.");
  if (!lines.length || lines.length > 100)
    throw new Error("Import 1–100 rows per batch.");
  return lines.map((values, index) => {
    if (values.length !== headers.length)
      throw new Error(`CSV row ${index + 2} has the wrong number of columns.`);
    return Object.fromEntries(headers.map((key, i) => [key, values[i] ?? ""]));
  });
}

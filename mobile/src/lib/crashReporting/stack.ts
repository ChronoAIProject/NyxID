import type { JsFrame } from "../../../../shared/crash-report";
export function parseMobileStack(stack: unknown): JsFrame[] {
  if (typeof stack !== "string") return [];
  return stack
    .slice(0, 65536)
    .split("\n")
    .slice(0, 128)
    .flatMap((line) => {
      const match = line.match(
        /(?:@|\s|\()(?:address at )?([^\s()]+):(\d+):(\d+)\)?\s*$/,
      );
      if (!match) return [];
      const lineno = Number(match[2]),
        column = Number(match[3]);
      if (
        lineno > 1e7 ||
        column > 1e7 ||
        !Number.isSafeInteger(lineno) ||
        !Number.isSafeInteger(column)
      )
        return [];
      const name = match[1]?.split(/[/?#]/).pop();
      const asset =
        name === "main.jsbundle" || name === "index.android.bundle"
          ? name
          : "<external>";
      return [{ asset, line: lineno, column }];
    })
    .slice(0, 64);
}

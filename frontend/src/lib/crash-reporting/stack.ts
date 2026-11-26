import type { JsFrame } from "../../../../shared/crash-report";
/** Parse locations only. Neither function names nor raw URL text enter the result. */
export function parseWebStack(stack: unknown, origin: string): JsFrame[] {
  if (typeof stack !== "string") return [];
  return stack
    .slice(0, 65536)
    .split("\n")
    .slice(0, 128)
    .flatMap((line) => {
      const match = line.match(
        /(?:\(|@|\s)((?:https?:\/\/|file:|blob:|data:|chrome-extension:|moz-extension:|<anonymous>)[^\s]*?):(\d+):(\d+)\)?\s*$/,
      );
      if (!match) return [];
      const lineno = Number(match[2]),
        column = Number(match[3]);
      if (
        !Number.isSafeInteger(lineno) ||
        !Number.isSafeInteger(column) ||
        lineno > 1e7 ||
        column > 1e7
      )
        return [];
      let asset = "<external>";
      if (match[1] === "<anonymous>") asset = "<inline>";
      else {
        try {
          const url = new URL(match[1]!);
          if (
            url.origin === origin &&
            !url.username &&
            !url.password &&
            /^\/assets\/[A-Za-z0-9_.-]{1,96}\.js$/.test(url.pathname)
          )
            asset = url.pathname.slice(1);
        } catch {
          /* Invalid locations remain external. */
        }
      }
      return [{ asset, line: lineno, column }];
    })
    .slice(0, 64);
}

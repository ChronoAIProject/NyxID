import { appI18n } from "@/i18n/instance";
import english from "@/i18n/locales/errors-en.json";
import type { TOptions } from "i18next";

const messages = new Set(Object.keys(english));
const envelopePrefix =
  /^(Bad request|Unauthorized|Forbidden|Not found|Conflict|Validation error|Authentication failed): ([\s\S]+)$/;

function translateKnown(message: string, options?: TOptions): string {
  return appI18n.t(message, {
    ...options,
    ns: "errors",
    keySeparator: false,
    nsSeparator: false,
    defaultValue: message,
  });
}

function translateBare(message: string, options?: TOptions): string {
  if (messages.has(message)) return translateKnown(message, options);

  const status = /^Request failed with status (\d{3})$/.exec(message);
  if (status) {
    return appI18n.t("Request failed with status {{status}}", {
      ...options,
      ns: "errors",
      keySeparator: false,
      nsSeparator: false,
      status: status[1],
      defaultValue: message,
    });
  }

  const envelope = envelopePrefix.exec(message);
  if (!envelope) return message;
  const detail = envelope[2]!;
  return appI18n.t(`${envelope[1]}: {{message}}`, {
    ...options,
    ns: "errors",
    keySeparator: false,
    nsSeparator: false,
    message: messages.has(detail) ? translateKnown(detail, options) : detail,
    defaultValue: message,
  });
}

/** Translate known error copy at presentation; retain raw errors for control flow. */
export function translateErrorMessage(
  message: string,
  options?: TOptions,
): string;
export function translateErrorMessage<T>(message: T, options?: TOptions): T;
export function translateErrorMessage(
  message: unknown,
  options?: TOptions,
): unknown {
  if (typeof message !== "string") return message;
  const trace = /\nTrace ID: ([a-zA-Z0-9_-]{8,128})$/.exec(message);
  const body = trace ? message.slice(0, trace.index) : message;
  const diagnostic = /^([\s\S]+) \((?:([^()\n]+) · )?code (\d+)\)$/.exec(body);
  const base = diagnostic ? diagnostic[1]! : body;
  let displayed = translateBare(base, options);
  if (diagnostic) {
    displayed =
      displayed === base
        ? body
        : appI18n.t(
            diagnostic[2]
              ? "{{message}} ({{hint}} · code {{code}})"
              : "{{message}} (code {{code}})",
            {
              ...options,
              ns: "errors",
              keySeparator: false,
              nsSeparator: false,
              message: displayed,
              hint: diagnostic[2],
              code: diagnostic[3],
              defaultValue: body,
            },
          );
  }
  if (!trace) return displayed;
  return `${displayed}\n${appI18n.t("Trace ID: {{id}}", {
    ...options,
    ns: "errors",
    keySeparator: false,
    nsSeparator: false,
    id: trace[1],
  })}`;
}

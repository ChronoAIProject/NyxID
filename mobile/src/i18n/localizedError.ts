import { translateApp } from './appTranslation';

/** The Error owns the source phrase and values for its lifetime. Reading its
 * message after a language change formats that same source again. */
export function localizedError(phrase: string, values?: Record<string, unknown>): Error {
  const error = new Error();
  // Materialize lazy runtime stack text before installing a display getter.
  // Diagnostics can later inspect frames without evaluating localized copy.
  void error.stack;
  const snapshot = values ? { ...values } : undefined;
  Object.defineProperty(error, 'message', { configurable: true, get: () => translateApp(phrase, snapshot) });
  return error;
}

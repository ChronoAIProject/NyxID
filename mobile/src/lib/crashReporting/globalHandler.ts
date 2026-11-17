export type ErrorHandler = (error: unknown, isFatal?: boolean) => void;
export type ErrorUtilsLike = {
  getGlobalHandler(): ErrorHandler;
  setGlobalHandler(handler: ErrorHandler): void;
};
export function chainGlobalHandler(
  utils: ErrorUtilsLike,
  fatal: (error: unknown) => void,
  nonfatal: (error: unknown) => void,
): () => void {
  const previous = utils.getGlobalHandler();
  const handler: ErrorHandler = (error, isFatal) => {
    try {
      if (isFatal) fatal(error);
      else nonfatal(error);
    } finally {
      previous(error, isFatal);
    }
  };
  utils.setGlobalHandler(handler);
  return () => {
    if (utils.getGlobalHandler() === handler) utils.setGlobalHandler(previous);
  };
}
export type RejectionOptions = {
  allRejections?: boolean;
  onUnhandled?: (id: number, error: unknown) => void;
  onHandled?: (id: number) => void;
};
/** Release tracking captures only; development retains RN diagnostics. */
export function chainRejections(
  previous: RejectionOptions,
  capture: (error: unknown) => void,
  development: boolean,
): RejectionOptions {
  if (!development) return {
    allRejections: true,
    onUnhandled(_id, error) { capture(error); },
    onHandled() {},
  };
  return {
    ...previous,
    allRejections: true,
    onUnhandled(id, error) {
      try {
        capture(error);
      } finally {
        previous.onUnhandled?.(id, error);
      }
    },
    onHandled: previous.onHandled,
  };
}

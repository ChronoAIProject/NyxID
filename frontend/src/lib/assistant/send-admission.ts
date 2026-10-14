/**
 * A send the server admitted (its POST returned 2xx) before the client lost
 * the reply. The server already stored the user message and bound its uploads,
 * so the composer must not restore the text or attachments: resending would
 * duplicate the message, and bound uploads cannot be bound again.
 */
export type AdmittedSendError = Error & { readonly admitted: true };

export function markAdmitted(error: unknown): AdmittedSendError {
  const marked =
    error instanceof Error
      ? error
      : new Error(typeof error === "string" ? error : "The assistant reply was interrupted.");
  return Object.assign(marked, { admitted: true as const });
}

export function isAdmittedSendError(error: unknown): error is AdmittedSendError {
  return error instanceof Error && (error as Partial<AdmittedSendError>).admitted === true;
}

/** Safe, short copy for failures without an actionable application message. */
export function assistantRequestErrorMessage(status: number, message?: string): string {
  if (status === 0) {
    return "Connection interrupted. Check your connection and the chat before trying again; your message may have arrived.";
  }
  if (status === 502 || status === 503) return "NyxBot is temporarily unavailable. Try again shortly.";
  if (status === 504 || status === 408) return "NyxBot took too long to respond. Check the chat before trying again.";
  if (status >= 500) return "Something went wrong while handling your request. Try again shortly.";
  if (status === 429) return "Too many requests. Wait a moment, then try again.";
  // Preserve useful validation and permission guidance, but replace opaque
  // transport messages. Server failures above never expose upstream details.
  if (message && !/^(network error|failed to fetch|load failed|internal server error|request failed with status \d+)\.?$/i.test(message.trim())) return message;
  if (status === 401) return "Your session or assistant connection needs to be renewed. Refresh the page and try again.";
  if (status === 403) return "You don't have permission for this action.";
  if (status === 404) return "This chat or resource is no longer available. Refresh the page and try again.";
  return "The request could not be completed. Check your input and try again.";
}

export function withAssistantTrace(message: string, traceId: string | null): string {
  // Only display bounded opaque IDs, never arbitrary header contents.
  return traceId && /^[a-zA-Z0-9_-]{8,128}$/.test(traceId)
    ? `${message}\nTrace ID: ${traceId}`
    : message;
}

/** Toasts carry the explanation; the inline notice retains the support ID. */
export function assistantErrorSummary(message: string): string {
  return message.replace(/\nTrace ID: [a-zA-Z0-9_-]{8,128}$/, "");
}

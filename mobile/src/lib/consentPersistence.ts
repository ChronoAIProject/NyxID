import type { createAnalyticsConsentState } from './analyticsConsentState';
/** Serial device writes with account/revision-bound failure handling. */
export function createConsentPersistence(state: ReturnType<typeof createAnalyticsConsentState>, write: (serialized: string) => Promise<void>) {
  let writes = Promise.resolve();
  return () => {
    const intent = state.persistenceIntent();
    if (!intent) return writes;
    writes = writes.then(async () => {
      try {
        // Serialize when this operation runs so prior failures' denials cannot
        // be overwritten by an already-queued snapshot from another account.
        await write(state.serialize());
        state.finishPersistence(intent, true);
      } catch { state.finishPersistence(intent, false); }
    });
    return writes;
  };
}

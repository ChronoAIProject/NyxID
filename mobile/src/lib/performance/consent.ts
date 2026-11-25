// A grant is local to this installation and destination. A new grant never
// authorizes an interaction that began under an earlier choice.
export function createPerformanceConsent(
  destination: string | null,
  read: () => Promise<string | null>,
  write: (value: string | null) => Promise<void>,
  sample: () => boolean,
) {
  let epoch = 0;
  let enabled = false;
  let initialGrant: number | null = null;
  let hydration: Promise<void> | undefined;
  let writes = Promise.resolve();
  const listeners = new Set<() => void>();
  const current = () => (enabled ? epoch : null);
  function publish(next: boolean) {
    enabled = next;
    for (const listener of listeners) listener();
  }
  function hydrate() {
    if (!destination || epoch !== 0) return Promise.resolve();
    return (hydration ??= read()
      .then((value) => {
        if (epoch === 0 && value === destination) {
          initialGrant = ++epoch;
          publish(true);
        }
      })
      .catch((error: unknown) => {
        hydration = undefined;
        throw error;
      }));
  }
  return {
    current,
    async coldGrant() {
      await hydrate();
      return current() === initialGrant ? initialGrant : null;
    },
    hydrate,
    begin() {
      return enabled && sample() ? epoch : null;
    },
    set(next: boolean) {
      if (!destination) return Promise.resolve();
      initialGrant = null;
      const revision = ++epoch;
      publish(false);
      // Serialize writes so a superseded grant cannot survive a later withdrawal on disk.
      const saved = writes.then(() => write(next ? destination : null));
      writes = saved.catch(() => {});
      return saved.then(() => {
        if (next && revision === epoch) publish(true);
      });
    },
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

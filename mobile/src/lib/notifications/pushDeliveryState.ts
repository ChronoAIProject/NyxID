let suppressed = false;
let hydration = Promise.resolve();
let hydrated = true;

export function setPushDeliverySuppressed(value: boolean): void {
  suppressed = value;
}

export function isPushDeliverySuppressed(): boolean {
  return suppressed;
}

export function beginPushDeliveryStateHydration(
  operation: Promise<unknown>,
): void {
  hydrated = false;
  hydration = operation.then(
    () => {
      hydrated = true;
    },
    () => {
      hydrated = true;
    },
  );
}

export function pushDeliveryStateHydrated(): Promise<void> {
  return hydration;
}

export function isPushDeliveryStateHydrated(): boolean {
  return hydrated;
}

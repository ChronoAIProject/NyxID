let suppressed = false;

export function setPushDeliverySuppressed(value: boolean): void {
  suppressed = value;
}

export function isPushDeliverySuppressed(): boolean {
  return suppressed;
}

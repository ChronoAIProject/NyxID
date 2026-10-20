/** Product analytics uses only authenticated NyxID relay; no SDK side channels,
 * anonymous identification, replay, lifecycle autocapture or raw exceptions. */
import Constants from 'expo-constants';
import { requestJson } from './api/http';
import { analyticsConsentState } from './analyticsConsentState';
import { createAnalyticsRelay } from './analyticsRelay';
import type { MobileEvent } from './telemetry-schema';
const relay = createAnalyticsRelay(analyticsConsentState.getSnapshot, requestJson);
export interface InitMobileTelemetryArgs { dsn: string | undefined; host: string | undefined; shareBack?: boolean; consent: boolean }
export async function initTelemetry(args: InitMobileTelemetryArgs): Promise<void> {
  // These settings only narrow collection. The backend owns the destination.
  relay.configure(args.consent && Boolean(args.dsn?.trim() || args.shareBack));
}
export function identify(userId: string): void { analyticsConsentState.bindAccount(userId || null); }
export function reset(): void { relay.configure(false); analyticsConsentState.bindAccount(null); }
export function shutdownTelemetry(): void { relay.configure(false); }
export function capture(event: MobileEvent): Promise<void> { return relay.capture(event); }
export function captureException(_error: unknown): void { /* No raw exception/stack export. */ }
export function readExpoTelemetryConfig(): { dsn: string | undefined; host: string | undefined; shareBack: boolean } {
  const extra = (Constants.expoConfig?.extra ?? {}) as Record<string, unknown>;
  return { dsn: typeof extra.TELEMETRY_DSN === 'string' ? extra.TELEMETRY_DSN : undefined,
    host: typeof extra.TELEMETRY_HOST === 'string' ? extra.TELEMETRY_HOST : undefined,
    shareBack: extra.NYXID_SHARE_ANALYTICS === true || extra.NYXID_SHARE_ANALYTICS === 'true' };
}

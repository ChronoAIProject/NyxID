import type { ToastState } from '../components/ToastOverlay';

/** Queue entries own their formatter until expiry; notification data remains verbatim. */
export function localizedToast(format: () => ToastState): ToastState {
  return { ...format(), format };
}

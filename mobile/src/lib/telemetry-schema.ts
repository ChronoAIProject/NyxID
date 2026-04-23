/**
 * Canonical mobile event schema — discriminated union of every named
 * event the mobile app emits. Mirrors the frontend's
 * `telemetry-schema.ts` pattern so adding a new variant is a
 * compile-time gate (no runtime validation needed).
 *
 * Covers two buckets:
 *   - `mobile.*` — device-side events the backend can't see
 *   - `ui.mobile_*` — CTA events using the shared §5.2 taxonomy with
 *     `mobile_` prefix
 *
 * See `docs/TELEMETRY.md` §5.3.
 */

// --- Narrow unions ---------------------------------------------------

export type MobileDomain =
  | 'auth'
  | 'approvals'
  | 'account'
  | 'settings'
  | 'activity'
  | 'legal'
  | 'nav';

export type MobileDestructiveAction =
  | 'delete_account'
  | 'revoke_session'
  | 'disconnect_device'
  | 'wipe'
  | 'cancel_registration';

export type MobileDecision = 'approve' | 'deny' | 'skip' | 'defer';

export type MobileConnectMethod = 'oauth' | 'device_code' | 'token_exchange' | 'password';

export type MobileProvider = 'google' | 'github' | 'apple';

export type LinkType = 'challenge' | 'approval' | 'other';

export type PushType = 'approval_request' | 'other';

export type AppState = 'foreground' | 'background';

export type BiometricReason = 'app_open' | 'approval_decision';

export type BiometricOutcome = 'success' | 'failed' | 'cancelled' | 'unavailable';

export type NavSource = 'tab' | 'header' | 'back' | 'deep_link';

export type MobileDialogId =
  | 'login'
  | 'register'
  | 'mfa_setup'
  | 'mfa_verify'
  | 'forgot_password'
  | 'biometric_enable'
  | 'approval_detail'
  | 'profile_edit'
  | 'change_password'
  | 'delete_account_confirm'
  | 'other';

// --- The discriminated union ----------------------------------------

export type MobileEvent =
  // --- `mobile.*` device-side events --------------------------------
  | {
      name: 'mobile.deep_link_opened';
      props: {
        link_type: LinkType;
      };
    }
  | {
      name: 'mobile.approval_viewed';
      props: {
        service_slug: string;
        mode: string;
      };
    }
  | {
      name: 'mobile.push_received';
      props: {
        type: PushType;
        app_state: AppState;
      };
    }
  | {
      name: 'mobile.biometric_prompted';
      props: {
        reason: BiometricReason;
      };
    }
  | {
      name: 'mobile.biometric_result';
      props: {
        reason: BiometricReason;
        outcome: BiometricOutcome;
      };
    }
  // --- `ui.mobile_*` CTA events -------------------------------------
  | {
      name: 'ui.mobile_dialog_opened';
      props: {
        dialog_id: MobileDialogId;
        entry_point: string;
      };
    }
  | {
      name: 'ui.mobile_dialog_step_completed';
      props: {
        dialog_id: MobileDialogId;
        step: number;
        total_steps: number;
      };
    }
  | {
      name: 'ui.mobile_dialog_abandoned';
      props: {
        dialog_id: MobileDialogId;
        final_step: number;
        duration_ms: number;
      };
    }
  | {
      name: 'ui.mobile_provider_connect_initiated';
      props: {
        provider: MobileProvider;
        method: MobileConnectMethod;
      };
    }
  | {
      name: 'ui.mobile_inline_edit_started';
      props: {
        domain: MobileDomain;
        field: string;
      };
    }
  | {
      name: 'ui.mobile_list_filtered';
      props: {
        list: string;
        filter: string;
        result_count: number;
      };
    }
  | {
      name: 'ui.mobile_nav_target_opened';
      props: {
        target: string;
        source: NavSource;
      };
    }
  | {
      name: 'ui.mobile_legal_page_opened';
      props: {
        page: 'privacy' | 'terms';
      };
    }
  | {
      name: 'ui.mobile_destructive_confirmed';
      props: {
        domain: MobileDomain;
        action: MobileDestructiveAction;
      };
    }
  | {
      name: 'ui.mobile_decision_made';
      props: {
        domain: MobileDomain;
        decision: MobileDecision;
        decision_ms: number;
      };
    }
  | {
      name: 'ui.mobile_preference_toggled';
      props: {
        name: string;
        value: string | boolean;
      };
    };

export const MOBILE_EVENT_NAMES = [
  'mobile.deep_link_opened',
  'mobile.approval_viewed',
  'mobile.push_received',
  'mobile.biometric_prompted',
  'mobile.biometric_result',
  'ui.mobile_dialog_opened',
  'ui.mobile_dialog_step_completed',
  'ui.mobile_dialog_abandoned',
  'ui.mobile_provider_connect_initiated',
  'ui.mobile_inline_edit_started',
  'ui.mobile_list_filtered',
  'ui.mobile_nav_target_opened',
  'ui.mobile_legal_page_opened',
  'ui.mobile_destructive_confirmed',
  'ui.mobile_decision_made',
  'ui.mobile_preference_toggled',
] as const satisfies readonly MobileEvent['name'][];

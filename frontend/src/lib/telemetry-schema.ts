/**
 * Canonical UI event schema — discriminated union of every named
 * `ui.*` event emitted from the frontend.
 *
 * Adding a new event = adding a variant. Unknown event names become a
 * compile-time error, which is how we enforce the allowlist called for
 * in `docs/TELEMETRY.md` §6 without any runtime validation.
 *
 * The shape of each event follows §5.2 taxonomy: 11 categories,
 * every CTA in the codebase fits into one of them (with a narrow
 * domain/action/status/etc. enum for the props).
 */

// --- Narrow string unions (enforced by TS; extend carefully) ----------

export type Domain =
  | 'keys'
  | 'services'
  | 'endpoints'
  | 'catalog'
  | 'api_keys'
  | 'agent_bindings'
  | 'approvals'
  | 'nodes'
  | 'channels'
  | 'notifications'
  | 'developer_apps'
  | 'oauth'
  | 'ssh'
  | 'auth'
  | 'mfa'
  | 'settings'
  | 'admin'
  | 'account'
  | 'nav'
  | 'docs';

export type SubDomain =
  | 'users'
  | 'roles'
  | 'groups'
  | 'service_accounts'
  | 'audit_log'
  | 'invite_codes'
  | 'orgs'
  | 'devices'
  | 'sessions'
  | 'profile'
  | 'mfa';

export type ActionStatus = 'confirmed' | 'cancelled' | 'errored';

export type DestructiveAction =
  | 'delete'
  | 'revoke'
  | 'rotate'
  | 'suspend'
  | 'unsuspend'
  | 'disconnect'
  | 'wipe';

export type Decision = 'approve' | 'deny' | 'skip' | 'defer';

export type ConnectMethod = 'oauth' | 'device_code' | 'api_key';

export type SecretType =
  | 'api_key'
  | 'client_id'
  | 'client_secret'
  | 'ca_key'
  | 'curl_example'
  | 'one_time_recovery_code'
  | 'other';

export type SecretContext = 'creation_modal' | 'detail_page' | 'settings';

export type NavSource = 'sidebar' | 'breadcrumb' | 'tab' | 'quick_link';

/**
 * `dialog_id` is a narrow enum so analytics has a bounded cardinality
 * of flow names. Add new wizards / dialogs here as they ship.
 */
export type DialogId =
  | 'login'
  | 'register'
  | 'forgot_password'
  | 'mfa_enroll'
  | 'mfa_verify'
  | 'add_key'
  | 'edit_key'
  | 'add_service'
  | 'edit_service'
  | 'add_endpoint'
  | 'edit_endpoint'
  | 'add_api_key'
  | 'edit_api_key'
  | 'add_agent_binding'
  | 'add_channel_bot'
  | 'edit_channel_bot'
  | 'add_node'
  | 'register_node_wizard'
  | 'add_developer_app'
  | 'edit_developer_app'
  | 'oauth_consent'
  | 'approval_mode_edit'
  | 'admin_suspend_user'
  | 'admin_edit_user'
  | 'admin_add_role'
  | 'admin_add_group'
  | 'admin_add_service_account'
  | 'admin_add_invite_code'
  | 'delete_account_confirm'
  | 'other';

// --- The discriminated union itself -----------------------------------

export type UiEvent =
  // § Dialog / wizard opened
  | {
      name: 'ui.dialog_opened';
      props: {
        dialog_id: DialogId;
        /** Which click-target or page opened this dialog. */
        entry_point: string;
      };
    }
  // § Flow step completed
  | {
      name: 'ui.dialog_step_completed';
      props: {
        dialog_id: DialogId;
        step: number;
        total_steps: number;
      };
    }
  // § Flow abandoned (closed without completion)
  | {
      name: 'ui.dialog_abandoned';
      props: {
        dialog_id: DialogId;
        final_step: number;
        duration_ms: number;
      };
    }
  // § Connection flow initiated (pre-OAuth-redirect intent)
  | {
      name: 'ui.provider_connect_initiated';
      props: {
        provider_slug: string;
        method: ConnectMethod;
      };
    }
  // § Sensitive copy-to-clipboard
  | {
      name: 'ui.secret_copied';
      props: {
        secret_type: SecretType;
        context: SecretContext;
      };
    }
  // § Inline edit entered (may be abandoned)
  | {
      name: 'ui.inline_edit_started';
      props: {
        domain: Domain;
        field: string;
      };
    }
  // § List filter / search applied
  | {
      name: 'ui.list_filtered';
      props: {
        list: string;
        filter: string;
        result_count: number;
      };
    }
  | {
      name: 'ui.list_searched';
      props: {
        list: string;
        filter: string;
        result_count: number;
      };
    }
  // § Navigation intent (beyond what autocapture can disambiguate)
  | {
      name: 'ui.nav_target_opened';
      props: {
        target: string;
        source: NavSource;
      };
    }
  // § External / docs link
  | {
      name: 'ui.docs_opened';
      props: {
        page: string;
      };
    }
  | {
      name: 'ui.external_link_opened';
      props: {
        url_domain: string;
      };
    }
  // § Destructive confirmation (delete/revoke/rotate/suspend/etc.)
  | {
      name: 'ui.destructive_confirmed';
      props: {
        domain: Domain;
        action: DestructiveAction;
        sub_domain?: SubDomain;
      };
    }
  // § Decision made (approve/deny, with view->tap latency)
  | {
      name: 'ui.decision_made';
      props: {
        domain: Domain;
        decision: Decision;
        decision_ms: number;
      };
    }
  // § Client-side preference toggle (theme / dense table / etc.)
  | {
      name: 'ui.preference_toggled';
      props: {
        name: string;
        value: string | boolean;
      };
    };

/**
 * Names of every `ui.*` event the schema recognizes. Useful for runtime
 * guards if/when we ever need them; primarily exported so CI-grep rules
 * can cross-check emission sites against the declared set.
 */
export const UI_EVENT_NAMES = [
  'ui.dialog_opened',
  'ui.dialog_step_completed',
  'ui.dialog_abandoned',
  'ui.provider_connect_initiated',
  'ui.secret_copied',
  'ui.inline_edit_started',
  'ui.list_filtered',
  'ui.list_searched',
  'ui.nav_target_opened',
  'ui.docs_opened',
  'ui.external_link_opened',
  'ui.destructive_confirmed',
  'ui.decision_made',
  'ui.preference_toggled',
] as const satisfies readonly UiEvent['name'][];

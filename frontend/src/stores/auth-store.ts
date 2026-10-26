import { useServiceCardView } from "@/stores/service-card-view-store";
import { create } from "zustand";
import type { User, LoginResponse } from "@/types/api";
import { api, apiClient, ApiError } from "@/lib/api-client";
import {
  identify as telemetryIdentify,
  reset as telemetryReset,
} from "@/lib/telemetry";
import { useAssistantContextStore } from "@/stores/assistant-context-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";

const MFA_REQUIRED_ERROR_CODE = 2002;

function clearAccountLocalState(): void {
  useServiceCardView.setState({
    accountId: undefined,
    filters: undefined,
    expanded: [],
    savedViewId: undefined,
  });
  useAssistantContextStore.getState().clear();
  useAssistantDraftStore.getState().clear();
  useAssistantWireLogStore.getState().reset();
}

function clearAccountIdentity({ resetTelemetry = false } = {}): void {
  // Reset attribution before publishing the cleared account state.
  if (resetTelemetry) telemetryReset();
  clearAccountLocalState();
  transitionAssistantIdentity(null);
}

function applyIdentityTransition(
  previousUser: User | null,
  nextUser: User | null,
): void {
  if (previousUser?.id === nextUser?.id) return;
  useServiceCardView.setState({
    accountId: nextUser?.id,
    filters: undefined,
    expanded: [],
    savedViewId: undefined,
  });
  if (previousUser !== null) clearAccountLocalState();
  transitionAssistantIdentity(nextUser?.id ?? null);
}

interface LoginResult {
  readonly mfaRequired: boolean;
  readonly response?: LoginResponse;
}

interface AuthState {
  readonly user: User | null;
  readonly isAuthenticated: boolean;
  readonly isLoading: boolean;
  readonly mfaRequired: boolean;
  readonly mfaToken: string | null;
}

interface AuthActions {
  readonly login: (email: string, password: string) => Promise<LoginResult>;
  readonly logout: () => Promise<void>;
  readonly checkAuth: (options?: {
    ephemeral?: boolean;
    /** QR session handoff: clear the unconfirmed identity and fence restoration. */
    isCurrent?: () => boolean;
  }) => Promise<boolean>;
  readonly setUser: (user: User | null) => void;
  readonly setMfaRequired: (required: boolean, token: string | null) => void;
  readonly clearMfaState: () => void;
}

type AuthStore = AuthState & AuthActions;

export const useAuthStore = create<AuthStore>((set, get) => ({
  user: null,
  isAuthenticated: false,
  isLoading: true,
  mfaRequired: false,
  mfaToken: null,

  login: async (email: string, password: string): Promise<LoginResult> => {
    try {
      const response = await api.post<LoginResponse>("/auth/login", {
        email,
        password,
        client: "web",
      });

      set({
        isAuthenticated: true,
        mfaRequired: false,
        mfaToken: null,
      });

      return { mfaRequired: false, response };
    } catch (error) {
      if (
        error instanceof ApiError &&
        error.errorCode === MFA_REQUIRED_ERROR_CODE
      ) {
        const sessionToken =
          (error.errorResponse as { session_token?: string }).session_token ??
          null;
        set({
          mfaRequired: true,
          mfaToken: sessionToken,
        });
        return { mfaRequired: true };
      }
      throw error;
    }
  },

  logout: async (): Promise<void> => {
    try {
      await api.post<void>("/auth/logout");
    } finally {
      clearAccountIdentity({ resetTelemetry: true });
      set({
        user: null,
        isAuthenticated: false,
        mfaRequired: false,
        mfaToken: null,
      });
    }
  },

  checkAuth: async ({
    ephemeral = false,
    isCurrent,
  } = {}): Promise<boolean> => {
    const authorityGuard = isCurrent
      ? () => {
          if (!isCurrent()) throw new Error("Profile restoration cancelled");
        }
      : undefined;
    if (isCurrent && !isCurrent()) return false;
    // Scoped QR restoration owns its loading UI and may be cancelled on unmount.
    if (isCurrent) {
      // The delivered cookie may belong to a different account. No old identity
      // is authoritative while its replacement profile is still unconfirmed.
      clearAccountIdentity({ resetTelemetry: true });
      set({ user: null, isAuthenticated: false });
    } else {
      set({ isLoading: true });
    }
    try {
      const user =
        ephemeral || authorityGuard
          ? await apiClient<User>("/users/me", {
              preserveSessionOn401: ephemeral,
              authorityGuard,
            })
          : await api.get<User>("/users/me");
      authorityGuard?.();
      applyIdentityTransition(get().user, user);
      set({
        user,
        isAuthenticated: true,
        ...(!isCurrent && { isLoading: false }),
        mfaRequired: false,
        mfaToken: null,
      });
      // Associate the restored session with its user_id so pageviews
      // captured after boot attribute correctly. Safe no-op when
      // telemetry is off.
      if (!ephemeral) telemetryIdentify(user.id);
      return true;
    } catch (error) {
      // Scoped failures retain pending MFA and leave global loading untouched.
      if (isCurrent) return false;
      if (error instanceof ApiError && error.status === 401) {
        // Session expired / server rejected — this is a sign-out path
        // and must reset the telemetry identity before we wipe state
        // so subsequent pageviews attribute to a fresh anon, not the
        // ex-user. Parity with the explicit `logout()` branch above.
        if (!ephemeral) {
          clearAccountIdentity({ resetTelemetry: true });
        } else {
          transitionAssistantIdentity(null);
        }
        set({ user: null, isAuthenticated: false, isLoading: false });
      } else {
        set({ isLoading: false });
      }
      return false;
    }
  },

  setUser: (user: User | null): void => {
    const previousUser = get().user;
    if (user === null) {
      clearAccountIdentity();
    } else {
      applyIdentityTransition(previousUser, user);
    }
    set({ user, isAuthenticated: user !== null });
    if (user !== null) {
      telemetryIdentify(user.id);
    }
  },

  setMfaRequired: (required: boolean, token: string | null): void => {
    set({ mfaRequired: required, mfaToken: token });
  },

  clearMfaState: (): void => {
    set({ mfaRequired: false, mfaToken: null });
  },
}));

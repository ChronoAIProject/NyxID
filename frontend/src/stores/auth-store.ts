import { useConsentStore } from "@/stores/consent-store";
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
  useServiceCardView.setState({ accountId: undefined, filters: undefined, expanded: [], savedViewId: undefined });
  useAssistantContextStore.getState().clear();
  useAssistantDraftStore.getState().clear();
  useAssistantWireLogStore.getState().reset();
}

function applyIdentityTransition(
  previousUser: User | null,
  nextUser: User | null,
): void {
  useConsentStore.getState().bindAccount(nextUser?.id ?? null);
  if (previousUser?.id === nextUser?.id) return;
  telemetryReset();
  useServiceCardView.setState({ accountId: nextUser?.id, filters: undefined, expanded: [], savedViewId: undefined });
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
  readonly checkAuth: (options?: { ephemeral?: boolean; requireSession?: boolean }) => Promise<void>;
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
      // Clear local analytics identity and pending admissions before state wipe.
      telemetryReset();
      useConsentStore.getState().bindAccount(null);
      clearAccountLocalState();
      transitionAssistantIdentity(null);
      set({
        user: null,
        isAuthenticated: false,
        mfaRequired: false,
        mfaToken: null,
      });
    }
  },

  checkAuth: async ({ ephemeral = false, requireSession = false } = {}): Promise<void> => {
    set({ isLoading: true });
    try {
      const user = ephemeral
        ? await apiClient<User>("/users/me", { preserveSessionOn401: true })
        : await api.get<User>("/users/me");
      applyIdentityTransition(get().user, user);
      set({ user, isAuthenticated: true, isLoading: false });
      // Bind explicit events to the restored account; no anonymous/pageview
      // collection occurs. The relay authenticates admission and dispatch.
      if (!ephemeral) telemetryIdentify(user.id);
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) {
        // Session expired / server rejected — this is a sign-out path
        // and must reset the telemetry identity before we wipe state
        // so no later event retains the former account.
        if (!ephemeral) {
          telemetryReset();
          useConsentStore.getState().bindAccount(null);
          clearAccountLocalState();
        }
        transitionAssistantIdentity(null);
        set({ user: null, isAuthenticated: false, isLoading: false });
      } else {
        set({ isLoading: false });
      }
      if (requireSession) throw error;
    }
  },

  setUser: (user: User | null): void => {
    const previousUser = get().user;
    if (user === null) {
      telemetryReset();
      useConsentStore.getState().bindAccount(null);
      clearAccountLocalState();
      transitionAssistantIdentity(null);
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

import { useConsentStore } from "@/stores/consent-store";
import { useServiceCardView } from "@/stores/service-card-view-store";
import { create } from "zustand";
import type { User, LoginResponse } from "@/types/api";
import { api, apiClient, ApiError } from "@/lib/api-client";
import { bindAccountResourcePreferences } from "@/lib/account-resource-preferences";
import { captureAccountRequest, getAssistantIdentityRevision, isAccountAbort, transitionAssistantIdentity } from "@/lib/assistant/identity";
import {
  identify as telemetryIdentify,
  reset as telemetryReset,
} from "@/lib/telemetry";
import { useAssistantContextStore } from "@/stores/assistant-context-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAssistantPinsStore } from "@/stores/assistant-pins-store";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import { usePendingConnectStore } from "./pending-connect-store";
import { useOAuthPopupStore } from "./oauth-popup-store";
import { useMachinePaneStore } from "./machine-pane-store";

const MFA_REQUIRED_ERROR_CODE = 2002;

function clearAccountLocalState(): void {
  useServiceCardView.setState({
    accountId: undefined,
    filters: undefined,
    expanded: [],
    savedViewId: undefined,
  });
  usePendingConnectStore.setState({ attempts: {} });
  useOAuthPopupStore.setState({ attempt: null });
  useAssistantContextStore.getState().clear();
  useAssistantDraftStore.getState().clear();
  useAssistantWireLogStore.getState().reset();
}

function clearAccountIdentity(): void {
  transitionAssistantIdentity(null);
  bindAccountResourcePreferences(null);
  useMachinePaneStore.getState().bindAccount(null);
  // Reset attribution before publishing the cleared account state.
  telemetryReset();
  useConsentStore.getState().bindAccount(null);
  useAssistantPinsStore.getState().bindAccount(null);
  clearAccountLocalState();
}

function applyIdentityTransition(
  previousUser: User | null,
  nextUser: User | null,
): void {
  if (previousUser?.id !== nextUser?.id) transitionAssistantIdentity(nextUser?.id ?? null);
  useConsentStore.getState().bindAccount(nextUser?.id ?? null);
  useAssistantPinsStore.getState().bindAccount(nextUser?.id ?? null);
  if (previousUser?.id === nextUser?.id) return;
  bindAccountResourcePreferences(nextUser?.id ?? null);
  useMachinePaneStore.getState().bindAccount(nextUser?.id ?? null);
  telemetryReset();
  useServiceCardView.setState({
    accountId: nextUser?.id,
    filters: undefined,
    expanded: [],
    savedViewId: undefined,
  });
  if (previousUser !== null) clearAccountLocalState();
  useAssistantWireLogStore.getState().bindAccount(nextUser?.id ?? null);
  useAssistantDraftStore.getState().bindAccount(nextUser?.id ?? null);
  useAssistantContextStore.getState().bindAccount(nextUser?.id ?? null);
}

interface LoginResult {
  readonly mfaRequired: boolean;
  readonly response?: LoginResponse;
}

interface AuthState {
  readonly accountGeneration: number;
  readonly isIdentityVerifying: boolean;
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
    requireSession?: boolean;
    /** QR session handoff: clear the unconfirmed identity and fence restoration. */
    isCurrent?: () => boolean;
  }) => Promise<boolean>;
  readonly verifySession: (options?: { isCurrent?: () => boolean; signal?: AbortSignal }) => Promise<boolean>;
  readonly setUser: (user: User | null) => void;
  readonly setMfaRequired: (required: boolean, token: string | null) => void;
  readonly clearMfaState: () => void;
}

type AuthStore = AuthState & AuthActions;
let sessionVerification: { revision: number; promise: Promise<boolean> } | undefined;

export const useAuthStore = create<AuthStore>((set, get) => ({
  accountGeneration: 0,
  isIdentityVerifying: false,
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

      // Standalone pages need the confirmed profile and feature flags before
      // navigation; they may never mount a user query to populate the store.
      await get().checkAuth({ requireSession: true });

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
    const request = captureAccountRequest();
    try {
      await api.post<void>("/auth/logout");
    } finally {
      // An old logout must never clear a replacement session.
      if (!request.signal.aborted) {
        const previousUser = get().user;
        clearAccountIdentity();
        set({
          accountGeneration: get().accountGeneration + (previousUser ? 1 : 0),
          user: null,
          isAuthenticated: false,
          mfaRequired: false,
          mfaToken: null,
        });
      }
      request.dispose();
    }
  },

  checkAuth: async ({
    ephemeral = false,
    requireSession = false,
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
      const previousUser = get().user;
      clearAccountIdentity();
      set({
        user: null,
        isAuthenticated: false,
        accountGeneration: get().accountGeneration + (previousUser ? 1 : 0),
      });
    } else {
      set({ isLoading: true });
    }
    const request = captureAccountRequest();
    try {
      const user =
        ephemeral || authorityGuard
          ? await apiClient<User>("/users/me", {
              preserveSessionOn401: ephemeral,
              authorityGuard,
            })
          : await api.get<User>("/users/me");
      request.assertCurrent();
      authorityGuard?.();
      const previousUser = get().user;
      applyIdentityTransition(previousUser, user);
      set({
        accountGeneration:
          get().accountGeneration +
          (previousUser && previousUser.id !== user.id ? 1 : 0),
        user,
        isAuthenticated: true,
        ...(!isCurrent && { isLoading: false }),
        mfaRequired: false,
        mfaToken: null,
      });
      // Bind explicit events to the freshly restored account.
      if (!ephemeral) telemetryIdentify(user.id);
      return true;
    } catch (error) {
      if (error instanceof Error && error.name === "AbortError") {
        if (!isCurrent) set({ isLoading: false });
        if (requireSession) throw error;
        return false;
      }
      // Scoped failures retain pending MFA and leave global loading untouched.
      if (isCurrent) {
        if (requireSession) throw error;
        return false;
      }
      if (error instanceof ApiError && error.status === 401) {
        // Session expired / server rejected — this is a sign-out path
        // and must reset the telemetry identity before we wipe state
        // so no later event retains the former account.
        if (!ephemeral) {
          clearAccountIdentity();
        } else {
          transitionAssistantIdentity(null);
          bindAccountResourcePreferences(null);
          useMachinePaneStore.getState().bindAccount(null);
          useAssistantPinsStore.getState().bindAccount(null);
          clearAccountLocalState();
        }
        set({
          user: null,
          isAuthenticated: false,
          isLoading: false,
          accountGeneration: get().accountGeneration + (get().user ? 1 : 0),
        });
      } else {
        set({ isLoading: false });
      }
      if (requireSession) throw error;
      return false;
    }
  },

  verifySession: ({ isCurrent, signal } = {}): Promise<boolean> => {
    const revision = getAssistantIdentityRevision();
    if (!isCurrent && sessionVerification?.revision === revision) return sessionVerification.promise;
    const controller = new AbortController();
    const abort = () => controller.abort(signal?.reason);
    signal?.addEventListener("abort", abort, { once: true });
    if (signal?.aborted) abort();
    const request = captureAccountRequest(controller.signal);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = new Promise<never>((_, reject) => {
      timer = setTimeout(() => {
        const error = new DOMException("Session verification timed out", "TimeoutError");
        controller.abort(error);
        reject(error);
      }, 15_000);
    });
    const promise = (async () => {
      try {
        const user = await Promise.race([
          apiClient<User>("/users/me", { preserveSessionOn401: true, signal: controller.signal, accountRequest: request }),
          deadline,
        ]);
        request.assertCurrent();
        if (isCurrent && !isCurrent()) return false;
        const previous = get().user;
        if (previous?.id === user.id) {
          // Idempotent binds retry a previous browser-storage read failure.
          applyIdentityTransition(previous, previous);
          if (JSON.stringify(previous) !== JSON.stringify(user)) set({ user });
        } else get().setUser(user);
        return true;
      } catch (error) {
        if ((isCurrent && !isCurrent()) || controller.signal.aborted || isAccountAbort(error) || revision !== getAssistantIdentityRevision()) return false;
        request.assertCurrent();
        if (error instanceof ApiError && error.status === 401) {
          if (get().user) get().setUser(null);
          return true;
        }
        return false;
      } finally { clearTimeout(timer); signal?.removeEventListener("abort", abort); request.dispose(); }
    })().finally(() => {
      if (sessionVerification?.promise === promise) sessionVerification = undefined;
    });
    if (!isCurrent) sessionVerification = { revision, promise };
    return promise;
  },

  setUser: (user: User | null): void => {
    const previousUser = get().user;
    if (user === null) {
      clearAccountIdentity();
    } else {
      applyIdentityTransition(previousUser, user);
    }
    set({
      user,
      isAuthenticated: user !== null,
      accountGeneration:
        get().accountGeneration +
        (previousUser && previousUser.id !== user?.id ? 1 : 0),
    });
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

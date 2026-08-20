import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  AUTH_DEVICE_ERROR_MESSAGES,
  approveBodySchema,
  approveResponseSchema,
  denyBodySchema,
  friendlyAuthDeviceErrorMessage,
  pollBodySchema,
  pollWebResponseSchema,
  previewResponseSchema,
  requestBodySchema,
  requestResponseSchema,
  type ApproveAuthDeviceResponse,
  type AuthDeviceRequestResponse,
  type PreviewAuthDeviceResponse,
} from "@/schemas/auth-device";

export function usePreviewAuthDevice() {
  return useMutation({
    mutationFn: async (
      userCode: string,
    ): Promise<PreviewAuthDeviceResponse> => {
      const response = await api.post<PreviewAuthDeviceResponse>(
        "/auth/device/preview",
        { user_code: userCode },
      );
      return previewResponseSchema.parse(response);
    },
  });
}

export function useApproveAuthDevice() {
  return useMutation({
    mutationFn: async (
      userCode: string,
    ): Promise<ApproveAuthDeviceResponse> => {
      const body = approveBodySchema.parse({ user_code: userCode });
      const response = await api.post<ApproveAuthDeviceResponse>(
        "/auth/device/approve",
        body,
      );
      return approveResponseSchema.parse(response);
    },
  });
}

export function useDenyAuthDevice() {
  return useMutation({
    mutationFn: async (
      userCode: string,
    ): Promise<ApproveAuthDeviceResponse> => {
      const body = denyBodySchema.parse({ user_code: userCode });
      const response = await api.post<ApproveAuthDeviceResponse>(
        "/auth/device/deny",
        body,
      );
      return approveResponseSchema.parse(response);
    },
  });
}

export type WebAuthDevicePhase =
  | "idle"
  | "requesting"
  | "pending"
  | "success"
  | "denied"
  | "expired"
  | "used"
  | "error";

interface WebAuthDeviceError {
  readonly code: number | null;
  readonly message: string;
}

export interface WebAuthDeviceLoginState {
  readonly phase: WebAuthDevicePhase;
  readonly request: AuthDeviceRequestResponse | null;
  readonly remainingSeconds: number | null;
  readonly error: WebAuthDeviceError | null;
}

const POLL_SLOW_DOWN_INCREMENT_SECONDS = 5;
const ACTION_THROTTLE_MS = 750;

function getApiErrorCode(error: unknown): number | null {
  if (
    typeof error === "object" &&
    error !== null &&
    "errorCode" in error &&
    typeof error.errorCode === "number"
  ) {
    return error.errorCode;
  }
  return null;
}

function getTerminalPhase(errorCode: number | null): WebAuthDevicePhase | null {
  switch (errorCode) {
    case 11204:
      return "denied";
    case 11200:
    case 11201:
      return "expired";
    case 11205:
      return "used";
    default:
      return null;
  }
}

function browserClientLabel(): string {
  const browserNavigator =
    typeof navigator !== "undefined"
      ? (navigator as Navigator & {
          readonly userAgentData?: { readonly platform?: string };
        })
      : null;
  const platform =
    browserNavigator?.userAgentData?.platform
      ? browserNavigator.userAgentData.platform
      : browserNavigator
        ? browserNavigator.platform
        : "browser";
  return `NyxID web (${platform || "browser"})`;
}

/**
 * Owns the browser-facing device-code request and polling lifecycle. The
 * device code remains in this hook's memory and is never persisted or logged.
 */
export function useWebAuthDeviceLogin(): WebAuthDeviceLoginState & {
  readonly start: () => void;
  readonly generateNew: () => void;
  readonly close: () => void;
} {
  const checkAuth = useAuthStore((state) => state.checkAuth);
  const [phase, setPhase] = useState<WebAuthDevicePhase>("idle");
  const [request, setRequest] =
    useState<AuthDeviceRequestResponse | null>(null);
  const [remainingSeconds, setRemainingSeconds] = useState<number | null>(null);
  const [error, setError] = useState<WebAuthDeviceError | null>(null);
  const phaseRef = useRef<WebAuthDevicePhase>("idle");
  const requestRef = useRef<AuthDeviceRequestResponse | null>(null);
  const intervalRef = useRef(5);
  const expiresAtRef = useRef<number | null>(null);
  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastActionAtRef = useRef(0);
  const stoppedRef = useRef(true);

  const clearPollTimer = useCallback(() => {
    if (pollTimerRef.current !== null) {
      clearTimeout(pollTimerRef.current);
      pollTimerRef.current = null;
    }
  }, []);

  const stopPolling = useCallback(() => {
    stoppedRef.current = true;
    clearPollTimer();
  }, [clearPollTimer]);

  const setCurrentPhase = useCallback((next: WebAuthDevicePhase) => {
    phaseRef.current = next;
    setPhase(next);
  }, []);

  const schedulePollRef = useRef<(delaySeconds: number) => void>(() => {});
  const poll = useCallback(async () => {
    if (stoppedRef.current || phaseRef.current !== "pending") return;
    const activeRequest = requestRef.current;
    if (!activeRequest) return;

    const expiresAt = expiresAtRef.current;
    if (expiresAt !== null && Date.now() >= expiresAt) {
      stopPolling();
      setCurrentPhase("expired");
      setError({ code: 11201, message: AUTH_DEVICE_ERROR_MESSAGES[11201] ?? "This code has expired." });
      return;
    }

    try {
      const body = pollBodySchema.parse({
        device_code: activeRequest.device_code,
      });
      const response = await api.post<unknown>("/auth/device/poll-web", body);
      pollWebResponseSchema.parse(response);
      stopPolling();
      await checkAuth();
      setCurrentPhase("success");
      setError(null);
    } catch (pollError) {
      const code = getApiErrorCode(pollError);
      const terminalPhase = getTerminalPhase(code);
      if (terminalPhase !== null) {
        stopPolling();
        setCurrentPhase(terminalPhase);
        setError({
          code,
          message: friendlyAuthDeviceErrorMessage(pollError),
        });
        return;
      }

      if (code === 11202 || code === 11203) {
        if (code === 11203) {
          intervalRef.current += POLL_SLOW_DOWN_INCREMENT_SECONDS;
        }
        schedulePollRef.current(intervalRef.current);
        return;
      }

      stopPolling();
      setCurrentPhase("error");
      setError({ code, message: friendlyAuthDeviceErrorMessage(pollError) });
    }
  }, [checkAuth, setCurrentPhase, stopPolling]);

  useEffect(() => {
    schedulePollRef.current = (delaySeconds: number) => {
      clearPollTimer();
      if (stoppedRef.current || phaseRef.current !== "pending") return;
      pollTimerRef.current = setTimeout(() => {
        pollTimerRef.current = null;
        void poll();
      }, delaySeconds * 1000);
    };
  }, [clearPollTimer, poll]);

  const begin = useCallback(async () => {
    const now = Date.now();
    if (now - lastActionAtRef.current < ACTION_THROTTLE_MS) return;
    lastActionAtRef.current = now;
    stopPolling();
    requestRef.current = null;
    expiresAtRef.current = null;
    setRequest(null);
    setRemainingSeconds(null);
    setError(null);
    stoppedRef.current = false;
    setCurrentPhase("requesting");

    try {
      const body = requestBodySchema.parse({
        client_label: browserClientLabel(),
        client_user_agent:
          typeof navigator !== "undefined" ? navigator.userAgent : "browser",
      });
      const response = requestResponseSchema.parse(
        await api.post<unknown>("/auth/device/request", body),
      );
      requestRef.current = response;
      setRequest(response);
      intervalRef.current = response.interval;
      const expiresAt = Date.now() + response.expires_in * 1000;
      expiresAtRef.current = expiresAt;
      setRemainingSeconds(response.expires_in);
      setCurrentPhase("pending");
      schedulePollRef.current(response.interval);
    } catch (requestError) {
      stopPolling();
      setCurrentPhase("error");
      setError({
        code: getApiErrorCode(requestError),
        message: friendlyAuthDeviceErrorMessage(requestError),
      });
    }
  }, [setCurrentPhase, stopPolling]);

  useEffect(() => {
    if (phase !== "pending" || expiresAtRef.current === null) return;
    const updateCountdown = () => {
      const expiresAt = expiresAtRef.current;
      if (expiresAt === null) return;
      const seconds = Math.max(0, Math.ceil((expiresAt - Date.now()) / 1000));
      setRemainingSeconds(seconds);
      if (seconds === 0 && phaseRef.current === "pending") {
        stopPolling();
        setCurrentPhase("expired");
        setError({
          code: 11201,
          message: AUTH_DEVICE_ERROR_MESSAGES[11201] ?? "This code has expired.",
        });
      }
    };
    updateCountdown();
    const timer = setInterval(updateCountdown, 1000);
    return () => clearInterval(timer);
  }, [phase, setCurrentPhase, stopPolling]);

  useEffect(() => () => stopPolling(), [stopPolling]);

  const close = useCallback(() => {
    stopPolling();
    requestRef.current = null;
    expiresAtRef.current = null;
    setRequest(null);
    setRemainingSeconds(null);
    setError(null);
    setCurrentPhase("idle");
  }, [setCurrentPhase, stopPolling]);

  return {
    phase,
    request,
    remainingSeconds,
    error,
    start: () => void begin(),
    generateNew: () => void begin(),
    close,
  };
}

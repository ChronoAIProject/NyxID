import { requireOptionalNativeModule } from "expo";
import * as Crypto from "expo-crypto";
import Constants from "expo-constants";
import { AppState, Platform } from "react-native";
import { analyticsConsentState } from "../analyticsConsentState";
import { requestJson } from "../api/http";
import { createCrashLifecycle, type CrashNativeModule } from "./lifecycle";
import {
  chainGlobalHandler,
  chainRejections,
  type ErrorUtilsLike,
  type RejectionOptions,
} from "./globalHandler";
const native =
  requireOptionalNativeModule<CrashNativeModule>("NyxCrashReporter");
const artifact = Constants.expoConfig?.extra?.crashArtifactId;
export const crashLifecycle =
  native && Platform.OS !== "web"
    ? createCrashLifecycle({
        native,
        snapshot: analyticsConsentState.getSnapshot,
        request: (path, options) =>
          requestJson(path, {
            ...options,
            // A diagnostics refusal must not turn a proxy 401 into sign-out.
            invalidateSessionOnAuthFailure: false,
          }),
        uuid: Crypto.randomUUID,
        platform: Platform.OS === "ios" ? "ios" : "android",
        osVersion: () =>
          native.getAppIdentity().osVersion ??
          String(Platform.Version).split(".").slice(0, 2).join("."),
        artifactId: typeof artifact === "string" ? artifact : undefined,
      })
    : null;
let installed = false;
export function installMobileCrashReporting(): void {
  if (installed || !crashLifecycle) return;
  installed = true;
  const lifecycle = crashLifecycle;
  const utils = (
    globalThis as typeof globalThis & { ErrorUtils?: ErrorUtilsLike }
  ).ErrorUtils;
  if (utils)
    chainGlobalHandler(
      utils,
      (e) => lifecycle.fatal(e),
      (e) => {
        void lifecycle.capture(e);
      },
    );
  // RN 0.83's Hermes tracker is initialized only in development by RN itself.
  // Release uses capture-only callbacks; development preserves RN diagnostics.
  const previous = __DEV__ ? (
    require("react-native/Libraries/promiseRejectionTrackingOptions") as {
      default: RejectionOptions;
    }
  ).default : {};
  const options = chainRejections(previous, (e) => {
    void lifecycle.capture(e, "unhandled_rejection");
  }, __DEV__);
  const hermes = (
    globalThis as typeof globalThis & {
      HermesInternal?: {
        enablePromiseRejectionTracker?: (options: RejectionOptions) => void;
      };
    }
  ).HermesInternal;
  if (hermes?.enablePromiseRejectionTracker)
    hermes.enablePromiseRejectionTracker(options);
  else
    (
      require("promise/setimmediate/rejection-tracking") as {
        enable: (options: RejectionOptions) => void;
      }
    ).enable(options);
  analyticsConsentState.subscribe(() => {
    void lifecycle.refresh();
  });
  AppState.addEventListener("change", (state) => {
    if (state === "active") void lifecycle.refresh();
  });
}

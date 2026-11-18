/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_MOBILE_APP_URL?: string;
}

/** Injected by `define` in vite.config.ts. See `src/lib/chunk-recovery.ts`. */
declare const __BUILD_ID__: string

declare const __BUILD_ARTIFACT_ID__: string;
declare const __APP_VERSION__: string;

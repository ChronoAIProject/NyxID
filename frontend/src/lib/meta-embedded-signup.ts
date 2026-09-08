import { embeddedSignupEventSchema } from "@/schemas/channel-managed";

export interface FacebookSdk {
  init(options: {
    appId: string;
    version: string;
    cookie?: boolean;
    xfbml?: boolean;
  }): void;
  login(
    callback: (response: {
      authResponse?: { code?: string };
      status?: string;
    }) => void,
    options: {
      config_id: string;
      response_type: "code";
      override_default_response_type: true;
      extras: Record<string, unknown>;
    },
  ): void;
}

declare global {
  interface Window {
    FB?: FacebookSdk;
  }
}

let sdkPromise: Promise<FacebookSdk> | undefined;
export function loadFacebookSdk(): Promise<FacebookSdk> {
  if (window.FB) return Promise.resolve(window.FB);
  if (sdkPromise) return sdkPromise;
  sdkPromise = new Promise<FacebookSdk>((resolve, reject) => {
    const script = document.createElement("script");
    script.src = "https://connect.facebook.net/en_US/sdk.js";
    script.async = true;
    script.crossOrigin = "anonymous";
    const fail = () => {
      window.clearTimeout(timer);
      script.remove();
      reject(
        new Error(
          "Meta sign-in could not load. Check your connection or content blocker and retry.",
        ),
      );
    };
    const timer = window.setTimeout(fail, 15_000);
    script.onerror = fail;
    script.onload = () => {
      window.clearTimeout(timer);
      if (window.FB) resolve(window.FB);
      else fail();
    };
    document.head.appendChild(script);
  }).catch((error: unknown) => {
    sdkPromise = undefined;
    throw error;
  });
  return sdkPromise;
}

export function parseEmbeddedSignupEvent(event: MessageEvent) {
  if (event.origin !== "https://www.facebook.com") return null;
  try {
    const result = embeddedSignupEventSchema.safeParse(
      typeof event.data === "string" ? JSON.parse(event.data) : event.data,
    );
    return result.success ? result.data : null;
  } catch {
    return null;
  }
}

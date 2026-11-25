import PostHog from "posthog-react-native";

export interface PerformanceDestination {
  token: string;
  host: string;
}

// The regular SDK queue can retry on foreground even after optOut(). Send a
// single best-effort report per sampled interaction, without an offline queue.
export class PerformancePostHog extends PostHog {
  constructor(destination: PerformanceDestination) {
    super(destination.token, {
      host: destination.host,
      persistence: "memory",
      captureAppLifecycleEvents: false,
      enableSessionReplay: false,
      errorTracking: { autocapture: false },
      disableSurveys: true,
      preloadFeatureFlags: false,
      disableRemoteConfig: true,
      setDefaultPersonProperties: false,
      personProfiles: "never",
      disableGeoip: true,
      fetchRetryCount: 0,
      disableCompression: true,
      requestTimeout: 5000,
    });
  }

  override getCommonEventProperties() {
    return {};
  }

  send(event: string, properties: Record<string, string | number>) {
    return this.captureStatelessImmediate(this.getAnonymousId(), event, {
      ...properties,
      $process_person_profile: false,
    });
  }
}

import { lazy, Suspense, type ComponentProps } from "react";

const VoicePanel = lazy(() =>
  import("./voice-panel").then((module) => ({ default: module.VoicePanel })),
);

/** Default-off voice must not load media setup code on ordinary chat visits. */
export function LazyVoicePanel(props: ComponentProps<typeof VoicePanel>) {
  return (
    <Suspense
      fallback={
        <p role="status" className="p-4 text-sm text-muted-foreground">
          Loading voice…
        </p>
      }
    >
      <VoicePanel {...props} />
    </Suspense>
  );
}

import { lazy, Suspense, type ComponentProps } from "react";
import { loadVoicePanel } from "@/lib/assistant/voice-panel-loader";

const VoicePanel = lazy(loadVoicePanel);

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

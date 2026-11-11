import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { lazy, Suspense, type ComponentProps } from "react";
import { loadVoicePanel } from "@/lib/assistant/voice-panel-loader";

const VoicePanel = lazy(loadVoicePanel);

/** Default-off voice must not load media setup code on ordinary chat visits. */
export function LazyVoicePanel(props: ComponentProps<typeof VoicePanel>) {
  useUiTranslation();
  return (
    <Suspense
      fallback={
        <p role="status" className="p-4 text-14 text-muted-foreground">{translateUi("Loading voice…")}</p>
      }
    >
      <VoicePanel {...props} />
    </Suspense>
  );
}

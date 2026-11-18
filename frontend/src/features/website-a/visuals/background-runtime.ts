import { visualConfig } from "./config";

export interface HeroScene {
  paused: boolean;
  destroy: () => void;
}
export interface BackgroundRuntime {
  addScene: (options: {
    projectId: string;
    element: HTMLElement;
    fps: number;
    dpi: number;
    interactivity: { mouse: { disabled: boolean } };
  }) => Promise<HeroScene>;
}
function existingRuntime(): BackgroundRuntime | undefined {
  return (window as Window & { UnicornStudio?: BackgroundRuntime })
    .UnicornStudio;
}
let runtime: Promise<BackgroundRuntime> | null = null;

export function loadBackgroundRuntime(): Promise<BackgroundRuntime> {
  const existing = existingRuntime();
  if (existing) return Promise.resolve(existing);
  if (runtime) return runtime;
  runtime = new Promise<BackgroundRuntime>((resolve, reject) => {
    const script = document.createElement("script");
    const finish = (error?: Error) => {
      window.clearTimeout(timeout);
      script.onload = null;
      script.onerror = null;
      const loaded = existingRuntime();
      if (error || !loaded) {
        script.remove();
        reject(error ?? new Error("Hero runtime unavailable"));
      } else resolve(loaded);
    };
    const timeout = window.setTimeout(
      () => finish(new Error("Hero runtime timed out")),
      8000,
    );
    script.src = visualConfig.backgroundUrl;
    script.async = true;
    script.onload = () => finish();
    script.onerror = () => finish(new Error("Hero runtime failed to load"));
    document.head.appendChild(script);
  }).catch((error: unknown) => {
    runtime = null;
    throw error;
  });
  return runtime;
}

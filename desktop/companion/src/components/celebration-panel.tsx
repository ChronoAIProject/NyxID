import { Mascot } from "./mascot";

interface CelebrationPanelProps {
  readonly companionName: string;
  readonly choice: string;
}

export function CelebrationPanel({
  companionName,
  choice,
}: CelebrationPanelProps) {
  return (
    <main className="panel celebration-panel" aria-live="polite">
      <div className="window-drag-strip" data-tauri-drag-region />
      <Mascot state="happy" size={176} />
      <p className="eyebrow">决定好了</p>
      <h1>{choice}</h1>
      <p>{companionName} 记住了，下次会更懂你的口味。</p>
    </main>
  );
}

import { BellOff, Settings, UtensilsCrossed } from "lucide-react";

import { IconButton } from "./icon-button";
import { Mascot } from "./mascot";

interface IdleViewProps {
  readonly name: string;
  readonly nextMeal: string;
  readonly quiet: boolean;
  readonly onOpen: () => void;
  readonly onSettings: () => void;
  readonly onToggleQuiet: () => void;
}

export function IdleView({
  name,
  nextMeal,
  quiet,
  onOpen,
  onSettings,
  onToggleQuiet,
}: IdleViewProps) {
  return (
    <main className="idle-view" data-tauri-drag-region>
      <div className="idle-bubble" aria-live="polite">
        <strong>{name}</strong>
        <span>{quiet ? "饭点提醒已暂停" : nextMeal}</span>
      </div>

      <button
        type="button"
        className="mascot-button"
        onClick={onOpen}
        aria-label={`让 ${name} 帮我选吃的`}
      >
        <Mascot state="idle" size={176} />
      </button>

      <div className="idle-actions">
        <IconButton label="现在选吃的" onClick={onOpen}>
          <UtensilsCrossed aria-hidden="true" />
        </IconButton>
        <IconButton
          label={quiet ? "恢复饭点提醒" : "暂停饭点提醒"}
          tone={quiet ? "active" : "default"}
          onClick={onToggleQuiet}
        >
          <BellOff aria-hidden="true" />
        </IconButton>
        <IconButton label="设置" onClick={onSettings}>
          <Settings aria-hidden="true" />
        </IconButton>
      </div>
    </main>
  );
}

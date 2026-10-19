import { useAccountPanel } from "@/hooks/use-account-panel";
import { Settings2 } from "lucide-react";

export function NyxBotSettingsButton() {
  const { open, current } = useAccountPanel();
  return (
    <button
      type="button"
      onClick={(event) => void open("nyxbot", {}, event.currentTarget)}
      aria-label="NyxBot settings"
      aria-haspopup="dialog"
      aria-expanded={current.panel === "nyxbot"}
      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-hairline text-text-tertiary transition-colors hover:border-hairline-strong hover:text-muted-foreground focus-visible:outline-none"
    >
      <Settings2 aria-hidden="true" className="h-[14px] w-[14px]" />
    </button>
  );
}

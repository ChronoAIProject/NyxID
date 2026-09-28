import { Bot } from "lucide-react";
import type { ServiceIconProps } from "./index";

export default function LlmNyxIcon({ className }: ServiceIconProps) {
  return (
    <span className={`relative inline-flex shrink-0 ${className ?? "h-5 w-5"}`}>
      <svg
        viewBox="0 0 424 424"
        className="h-full w-full"
        fill="currentColor"
        data-slug="llm-nyx"
        aria-hidden="true"
      >
        <path d="M422.875 88.0461V335.824C422.875 383.898 383.903 422.87 335.829 422.87H214.328C213.008 422.87 211.938 421.799 211.938 420.48V191.899C211.938 189.461 208.72 188.587 207.487 190.69L72.0088 421.69C71.5786 422.421 70.7947 422.87 69.9486 422.87H3.39006C2.07075 422.87 1 421.799 1 420.48V3.39006C1 2.07075 2.07075 1 3.39006 1H139.237C140.556 1 141.627 2.07075 141.627 3.39006V231.971C141.627 234.409 144.844 235.284 146.077 233.18L281.56 2.18069C281.99 1.44933 282.774 1 283.62 1H335.824C383.898 1 422.87 39.9724 422.87 88.0461H422.875Z" />
      </svg>
      <Bot
        aria-hidden="true"
        className="absolute bottom-[12%] right-[9%] !h-[35%] !w-[35%] text-background"
        strokeWidth={2.5}
      />
    </span>
  );
}

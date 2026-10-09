import type { CSSProperties } from "react";

import "./mascot.css";

export type MascotState = "idle" | "hungry" | "thinking" | "happy";

interface MascotProps {
  readonly state: MascotState;
  readonly size?: number;
  readonly label?: string;
}

const DEFAULT_LABELS: Record<MascotState, string> = {
  idle: "Nyx is resting",
  hungry: "Nyx says it is meal time",
  thinking: "Nyx is thinking",
  happy: "Nyx is happy",
};

export function Mascot({ state, size = 168, label }: MascotProps) {
  return (
    <div
      className={`mascot mascot--${state}`}
      style={{ "--mascot-size": `${String(size)}px` } as CSSProperties}
      role="img"
      aria-label={label ?? DEFAULT_LABELS[state]}
    >
      <svg
        viewBox="0 0 220 220"
        width={size}
        height={size}
        aria-hidden="true"
        focusable="false"
      >
        <defs>
          <linearGradient id="nyx-wing-left" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stopColor="#34323b" />
            <stop offset="1" stopColor="#201f25" />
          </linearGradient>
          <linearGradient id="nyx-wing-right" x1="1" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#34323b" />
            <stop offset="1" stopColor="#201f25" />
          </linearGradient>
          <filter id="nyx-shadow" x="-30%" y="-30%" width="160%" height="180%">
            <feDropShadow
              dx="0"
              dy="8"
              stdDeviation="7"
              floodColor="#0c0d10"
              floodOpacity="0.28"
            />
          </filter>
        </defs>

        <g className="mascot__float" filter="url(#nyx-shadow)">
          <ellipse
            className="mascot__ground"
            cx="110"
            cy="198"
            rx="55"
            ry="9"
          />

          <g className="mascot__tail">
            <path
              d="M151 157c26-6 42 4 39 20-2 12-16 19-33 14 12-2 18-8 17-15-1-8-10-10-22-6z"
              fill="#28272e"
            />
            <path
              d="M178 170c7 4 7 11 1 16 9-2 14-8 11-14-2-4-7-6-12-7z"
              fill="#f4eee5"
            />
          </g>

          <g className="mascot__wings">
            <path
              className="mascot__wing mascot__wing--left"
              d="M74 88C49 79 29 91 25 116c-4 27 18 45 48 39 10-2 19-7 25-15-21 3-32-5-35-19-3-13 3-24 17-31z"
              fill="url(#nyx-wing-left)"
            />
            <path
              className="mascot__wing mascot__wing--right"
              d="M146 88c25-9 45 3 49 28 4 27-18 45-48 39-10-2-19-7-25-15 21 3 32-5 35-19 3-13-3-24-17-31z"
              fill="url(#nyx-wing-right)"
            />
            <path
              d="M43 115c9 1 18 6 25 15M177 115c-9 1-18 6-25 15"
              className="mascot__wing-mark"
            />
          </g>

          <path
            className="mascot__body"
            d="M65 129c0-35 20-55 45-55s45 20 45 55v35c0 23-18 37-45 37s-45-14-45-37z"
            fill="#29282f"
          />

          <g className="mascot__ears">
            <path d="M72 87 66 37c0-7 7-10 12-5l25 31z" fill="#29282f" />
            <path d="m77 69-5-25 16 20z" fill="#7058d6" opacity="0.9" />
            <path d="m148 87 6-50c0-7-7-10-12-5l-25 31z" fill="#29282f" />
            <path d="m143 69 5-25-16 20z" fill="#7058d6" opacity="0.9" />
          </g>

          <path
            className="mascot__face"
            d="M74 93c5-25 21-39 36-39s31 14 36 39c4 21-9 40-36 40S70 114 74 93z"
            fill="#f4eee5"
          />

          <g className="mascot__eyes mascot__eyes--open">
            <ellipse cx="94" cy="95" rx="5" ry="7" fill="#202126" />
            <ellipse cx="126" cy="95" rx="5" ry="7" fill="#202126" />
            <circle cx="96" cy="92" r="1.5" fill="#fff" />
            <circle cx="128" cy="92" r="1.5" fill="#fff" />
          </g>
          <g className="mascot__eyes mascot__eyes--closed">
            <path d="M87 96c4 5 10 5 14 0M119 96c4 5 10 5 14 0" />
          </g>
          <g className="mascot__eyes mascot__eyes--thinking">
            <ellipse cx="94" cy="95" rx="5" ry="7" fill="#202126" />
            <ellipse cx="126" cy="95" rx="5" ry="7" fill="#202126" />
            <circle cx="96" cy="91" r="1.5" fill="#fff" />
            <circle cx="128" cy="91" r="1.5" fill="#fff" />
          </g>

          <path
            className="mascot__mouth mascot__mouth--idle"
            d="M105 111c3 3 7 3 10 0"
          />
          <path
            className="mascot__mouth mascot__mouth--hungry"
            d="M104 110c4-3 8-3 12 0-1 7-11 7-12 0z"
          />
          <path
            className="mascot__mouth mascot__mouth--thinking"
            d="M106 112h8"
          />
          <path
            className="mascot__mouth mascot__mouth--happy"
            d="M101 108c5 11 13 11 18 0"
          />

          <g className="mascot__scarf">
            <path
              d="M73 126c24 10 50 10 74 0l5 15c-26 11-58 11-84 0z"
              fill="#ef765e"
            />
            <path
              d="M136 135c9 8 14 21 11 36l-14-6c4-12 2-21-4-28z"
              fill="#df624f"
            />
            <circle cx="83" cy="137" r="5" fill="#57c6a5" />
            <path
              d="m83 133 1.2 2.5 2.8.4-2 2 .5 2.8-2.5-1.3-2.5 1.3.5-2.8-2-2 2.8-.4z"
              fill="#f4eee5"
            />
          </g>

          <g className="mascot__paws">
            <ellipse cx="91" cy="183" rx="16" ry="9" fill="#f4eee5" />
            <ellipse cx="129" cy="183" rx="16" ry="9" fill="#f4eee5" />
          </g>

          <g className="mascot__hungry-mark">
            <path d="M51 76v17M46 76v8c0 4 10 4 10 0v-8M65 76v17M65 76c8 3 8 11 0 13" />
          </g>
          <g className="mascot__thinking-mark">
            <path d="m171 65 3 7 7 3-7 3-3 7-3-7-7-3 7-3z" fill="#57c6a5" />
            <circle cx="157" cy="89" r="3" fill="#7058d6" />
          </g>
          <g className="mascot__happy-mark">
            <path
              d="m48 70 2.5 6 6 2.5-6 2.5-2.5 6-2.5-6-6-2.5 6-2.5z"
              fill="#ef765e"
            />
            <path d="m173 82 2 5 5 2-5 2-2 5-2-5-5-2 5-2z" fill="#57c6a5" />
          </g>
        </g>
      </svg>
    </div>
  );
}

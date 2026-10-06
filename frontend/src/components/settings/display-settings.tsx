import { useRef, useState, type ReactNode } from "react";
import { Check, RotateCcw, TriangleAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { useResolvedTheme } from "@/hooks/use-theme";
import {
  CONTRAST_CHECKS,
  EDITABLE_COLORS,
  contrastRatio,
  isHexColor,
  resolveColor,
  type ColorKey,
  type ColorMode,
} from "@/lib/theme-colors";
import { cn } from "@/lib/utils";
import {
  SIDEBAR_WIDTH,
  DISPLAY_SCALE_DEFAULT,
  DISPLAY_SCALE_MAX,
  DISPLAY_SCALE_MIN,
  DISPLAY_SCALE_STEP,
  useThemeStore,
  type SidebarId,
  type SidebarMode,
  type MotionPreference,
  type ThemeMode,
} from "@/stores/theme-store";

const THEME_OPTIONS: readonly { value: ThemeMode; label: string }[] = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

const MOTION_OPTIONS: readonly { value: MotionPreference; label: string }[] = [
  { value: "system", label: "Follow system" },
  { value: "reduce", label: "Reduce" },
];

const COLOR_MODE_OPTIONS: readonly { value: ColorMode; label: string }[] = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

function SegmentedChoice<T extends string | number>({
  label,
  options,
  value,
  onChange,
}: {
  readonly label: string;
  readonly options: readonly { value: T; label: string }[];
  readonly value: T;
  readonly onChange: (value: T) => void;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="flex flex-wrap gap-1 rounded-lg border border-hairline p-0.5">
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          aria-checked={value === option.value}
          onClick={() => onChange(option.value)}
          className={cn(
            "text-control h-7 rounded-md px-3 text-12 transition-colors",
            value === option.value
              ? "bg-overlay-strong font-medium text-foreground"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function SettingRow({
  title,
  description,
  children,
}: {
  readonly title: string;
  readonly description: string;
  readonly children: ReactNode;
}) {
  return (
    <div className="flex flex-col items-start justify-between gap-3 sm:flex-row sm:items-center">
      <div className="min-w-0 flex-1">
        <p className="text-12 font-medium">{title}</p>
        <p className="text-12 text-muted-foreground">{description}</p>
      </div>
      {children}
    </div>
  );
}

/** Hex text field that only commits complete `#RRGGBB` values. */
function HexField({
  label,
  value,
  onCommit,
}: {
  readonly label: string;
  readonly value: string;
  readonly onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [synced, setSynced] = useState(value);
  if (value !== synced) {
    setSynced(value);
    setDraft(value);
  }
  const normalized = draft.startsWith("#") ? draft : `#${draft}`;
  const invalid = !isHexColor(normalized);

  return (
    <Input
      aria-label={`${label} hex value`}
      aria-invalid={invalid || undefined}
      value={draft}
      maxLength={7}
      spellCheck={false}
      className="w-24 font-mono uppercase"
      onChange={(e) => {
        const next = e.target.value.trim();
        setDraft(next);
        const candidate = next.startsWith("#") ? next : `#${next}`;
        if (isHexColor(candidate)) onCommit(candidate);
      }}
      onBlur={() => {
        if (invalid) setDraft(value);
      }}
    />
  );
}

const SIDEBAR_MODE_OPTIONS: readonly { value: SidebarMode; label: string }[] = [
  { value: "expanded", label: "Expanded" },
  { value: "collapsed", label: "Collapsed" },
  { value: "hover", label: "Expand on hover" },
];

function SidebarWidthControl({ sidebar, label }: { readonly sidebar: SidebarId; readonly label: string }) {
  const width = useThemeStore((s) => s.sidebarWidths[sidebar]);
  const setSidebarWidth = useThemeStore((s) => s.setSidebarWidth);
  return (
    <div className="flex items-center gap-3">
      <input
        type="range"
        aria-label={label}
        min={SIDEBAR_WIDTH.min}
        max={SIDEBAR_WIDTH.max}
        step={4}
        value={width}
        onChange={(e) => setSidebarWidth(sidebar, Number(e.target.value))}
        className="w-40 accent-primary"
      />
      <span className="w-12 text-right font-mono text-11 text-muted-foreground">{width}px</span>
      <Button
        variant="ghost"
        size="icon"
        aria-label={`Reset ${label.toLowerCase()}`}
        title="Reset to default"
        disabled={width === SIDEBAR_WIDTH.default}
        onClick={() => setSidebarWidth(sidebar, SIDEBAR_WIDTH.default)}
        className={cn(width === SIDEBAR_WIDTH.default && "invisible")}
      >
        <RotateCcw className="size-3" />
      </Button>
    </div>
  );
}

function ScaleControl({ id, label, scale, onChange }: {
  readonly id: string;
  readonly label: string;
  readonly scale: number;
  readonly onChange: (value: number) => void;
}) {
  const [draft, setDraft] = useState<number | null>(null);
  const dragPointer = useRef<number | null>(null);
  const value = draft ?? scale;
  const multiplier = `${value}×`;
  const defaultPosition =
    ((DISPLAY_SCALE_DEFAULT - DISPLAY_SCALE_MIN) / (DISPLAY_SCALE_MAX - DISPLAY_SCALE_MIN)) * 100;

  return (
    <div className="flex w-full max-w-full flex-wrap items-center gap-3 sm:w-auto">
      <div className="flex w-64 max-w-full flex-1 items-start gap-3">
        <div className="min-w-0 flex-1 space-y-1">
          <input
            id={id}
            type="range"
            aria-label={label}
            aria-valuetext={multiplier}
            min={DISPLAY_SCALE_MIN}
            max={DISPLAY_SCALE_MAX}
            step={DISPLAY_SCALE_STEP}
            value={value}
            onChange={(e) => {
              const next = e.currentTarget.valueAsNumber;
              if (dragPointer.current !== null) setDraft(next);
              else onChange(next);
            }}
            onPointerDown={(e) => {
              if (e.button !== 0 || dragPointer.current !== null) return;
              dragPointer.current = e.pointerId;
              e.currentTarget.setPointerCapture(e.pointerId);
              setDraft(scale);
            }}
            onPointerUp={(e) => {
              if (dragPointer.current !== e.pointerId) return;
              dragPointer.current = null;
              onChange(e.currentTarget.valueAsNumber);
              setDraft(null);
            }}
            onPointerCancel={() => {
              dragPointer.current = null;
              setDraft(null);
            }}
            onLostPointerCapture={() => {
              dragPointer.current = null;
              setDraft(null);
            }}
            className="block h-6 w-full cursor-pointer rounded accent-primary focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-ring"
          />
          <div className="relative flex justify-between text-11 text-muted-foreground" aria-hidden="true">
            <span>{DISPLAY_SCALE_MIN}×</span>
            <span className="absolute -translate-x-1/2" style={{ left: `${defaultPosition}%` }}>{DISPLAY_SCALE_DEFAULT}×</span>
            <span>{DISPLAY_SCALE_MAX}×</span>
          </div>
        </div>
        <output htmlFor={id} aria-live="off" className="min-h-6 min-w-[5ch] shrink-0 text-right font-mono text-11 text-muted-foreground">
          {multiplier}
        </output>
      </div>
    </div>
  );
}

function SidebarSettings() {
  const sidebarMode = useThemeStore((s) => s.sidebarMode);
  const setSidebarMode = useThemeStore((s) => s.setSidebarMode);
  return (
    <Card>
      <CardHeader>
        <CardTitle>Sidebar</CardTitle>
        <CardDescription>
          Drag a sidebar&apos;s edge to resize it, or set the width here. Double-click the edge to reset.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-5">
        <SettingRow title="Main sidebar" description="Expanded, icons only, or icons that expand on hover.">
          <SegmentedChoice
            label="Main sidebar"
            options={SIDEBAR_MODE_OPTIONS}
            value={sidebarMode}
            onChange={setSidebarMode}
          />
        </SettingRow>
        <SettingRow title="Main sidebar width" description="Used when the main sidebar is expanded.">
          <SidebarWidthControl sidebar="dashboard" label="Main sidebar width" />
        </SettingRow>
        <SettingRow title="Assistant sidebar width" description="The agents and chats list in the assistant.">
          <SidebarWidthControl sidebar="assistant" label="Assistant sidebar width" />
        </SettingRow>
      </CardContent>
    </Card>
  );
}

function ColorEditor() {
  const resolved = useResolvedTheme();
  const setMode = useThemeStore((s) => s.setMode);
  const customColors = useThemeStore((s) => s.customColors);
  const setCustomColor = useThemeStore((s) => s.setCustomColor);
  const resetCustomColors = useThemeStore((s) => s.resetCustomColors);
  const [editOverride, setEditMode] = useState<ColorMode | null>(null);
  const editMode = editOverride ?? resolved;
  const overrides = customColors[editMode];
  const customizedCount = Object.keys(overrides).length;
  const color = (key: ColorKey) => resolveColor(editMode, overrides, key);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Colors</CardTitle>
        <CardDescription>
          Customize the colors of each theme. Changes apply immediately and are saved in this browser.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <SegmentedChoice
            label="Theme to edit"
            options={COLOR_MODE_OPTIONS}
            value={editMode}
            onChange={setEditMode}
          />
          {editMode !== resolved && (
            <Button variant="outline" size="sm" onClick={() => setMode(editMode)}>
              Switch to {editMode} theme to preview
            </Button>
          )}
        </div>

        <ul className="grid gap-x-8 gap-y-2 sm:grid-cols-2" aria-label={`${editMode} theme colors`}>
          {EDITABLE_COLORS.map(({ key, label }) => {
            const value = color(key);
            const customized = key in overrides;
            return (
              <li key={key} className="flex items-center gap-2">
                <span className="min-w-0 flex-1 text-12">
                  {label}
                  {customized && <span className="ml-2 text-11 text-text-tertiary">Customized</span>}
                </span>
                <input
                  type="color"
                  aria-label={`${label} color`}
                  value={value.toLowerCase()}
                  onChange={(e) => setCustomColor(editMode, key, e.target.value)}
                  className="h-7 w-9 cursor-pointer rounded-md border border-input bg-transparent p-0.5"
                />
                <HexField
                  label={label}
                  value={value}
                  onCommit={(next) => setCustomColor(editMode, key, next)}
                />
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`Reset ${label}`}
                  title="Reset to default"
                  disabled={!customized}
                  onClick={() => resetCustomColors(editMode, key)}
                  className={cn(!customized && "invisible")}
                >
                  <RotateCcw className="size-3" />
                </Button>
              </li>
            );
          })}
        </ul>

        <div className="space-y-1.5">
          <p className="text-12 font-medium">Readability</p>
          <ul className="space-y-1" aria-label="Contrast checks">
            {CONTRAST_CHECKS.map((check) => {
              const ratio = contrastRatio(color(check.fg), color(check.bg));
              const ok = ratio >= check.target;
              return (
                <li key={check.label} className="flex items-center gap-2 text-12">
                  {ok ? (
                    <Check className="size-3.5 shrink-0 text-success" aria-hidden />
                  ) : (
                    <TriangleAlert className="size-3.5 shrink-0 text-warning" aria-hidden />
                  )}
                  <span className="flex-1 text-muted-foreground">{check.label}</span>
                  <span className={cn("font-mono text-11", ok ? "text-muted-foreground" : "text-warning")}>
                    {ratio.toFixed(1)}:1 {ok ? "" : `(needs ${check.target}:1)`}
                  </span>
                </li>
              );
            })}
          </ul>
        </div>

        <div className="flex justify-end">
          <Button
            variant="outline"
            size="sm"
            disabled={customizedCount === 0}
            onClick={() => resetCustomColors(editMode)}
          >
            <RotateCcw className="size-3" />
            Reset {editMode} colors
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

export function DisplaySettings() {
  const mode = useThemeStore((s) => s.mode);
  const setMode = useThemeStore((s) => s.setMode);
  const textScale = useThemeStore((s) => s.textScale);
  const setTextScale = useThemeStore((s) => s.setTextScale);
  const density = useThemeStore((s) => s.density);
  const setDensity = useThemeStore((s) => s.setDensity);
  const motion = useThemeStore((s) => s.motion);
  const setMotion = useThemeStore((s) => s.setMotion);
  const resetDisplay = useThemeStore((s) => s.resetDisplay);

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Display</CardTitle>
          <CardDescription>
            Choose how NyxID looks in this browser. Other devices keep their own settings.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="space-y-5">
            <SettingRow title="Theme" description="System follows your device's light or dark setting.">
              <SegmentedChoice label="Theme" options={THEME_OPTIONS} value={mode} onChange={setMode} />
            </SettingRow>
            <SettingRow title="Text size" description="Scales all text in 0.01× steps.">
              <ScaleControl id="text-size" label="Text size" scale={textScale} onChange={setTextScale} />
            </SettingRow>
            <SettingRow title="Spacing" description="Adjust space between and inside elements in 0.01× steps.">
              <ScaleControl id="spacing" label="Spacing" scale={density} onChange={setDensity} />
            </SettingRow>
            <SettingRow title="Motion" description="Reduce turns off animations and transitions.">
              <SegmentedChoice label="Motion" options={MOTION_OPTIONS} value={motion} onChange={setMotion} />
            </SettingRow>
          </div>
          <div className="mt-4 flex justify-end">
            <Button variant="ghost" size="sm" onClick={resetDisplay}>
              <RotateCcw className="size-3" />
              Reset display settings
            </Button>
          </div>
        </CardContent>
      </Card>
      <SidebarSettings />
      <ColorEditor />
    </div>
  );
}

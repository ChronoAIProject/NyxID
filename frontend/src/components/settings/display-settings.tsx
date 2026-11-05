import { RangeControl } from "@/components/ui/range-control";
import { Label, FieldHelp } from "@/components/ui/label";
import { useState, type ReactNode } from "react";
import {
  Check,
  Monitor,
  Moon,
  RotateCcw,
  Sun,
  TriangleAlert,
  type LucideIcon,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  GroupedList,
  GroupedRow,
  GroupedInput as Input,
} from "@/components/ui/grouped-list";
import {
  ChatWidthControl,
  ComposerHeightControl,
} from "@/components/assistant/chat-width-control";
import { UserMessageBubble } from "@/components/ui/user-message-bubble";
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

const THEME_OPTIONS: readonly {
  value: ThemeMode;
  label: string;
  icon: LucideIcon;
}[] = [
  { value: "system", label: "System", icon: Monitor },
  { value: "light", label: "Light", icon: Sun },
  { value: "dark", label: "Dark", icon: Moon },
];

const MOTION_OPTIONS: readonly { value: MotionPreference; label: string }[] = [
  { value: "system", label: "Follow system" },
  { value: "reduce", label: "Reduce" },
];

const COLOR_MODE_OPTIONS: readonly {
  value: ColorMode;
  label: string;
  icon: LucideIcon;
}[] = [
  { value: "light", label: "Light", icon: Sun },
  { value: "dark", label: "Dark", icon: Moon },
];

function SegmentedChoice<T extends string | number>({
  label,
  options,
  value,
  onChange,
}: {
  readonly label: string;
  readonly options: readonly { value: T; label: string; icon?: LucideIcon }[];
  readonly value: T;
  readonly onChange: (value: T) => void;
}) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="flex max-w-full flex-wrap gap-1 rounded-lg bg-muted p-1"
    >
      {options.map(({ icon: Icon, ...option }, index) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          aria-checked={value === option.value}
          tabIndex={value === option.value ? 0 : -1}
          onClick={() => onChange(option.value)}
          onKeyDown={(event) => {
            const direction =
              event.key === "ArrowRight" || event.key === "ArrowDown"
                ? 1
                : event.key === "ArrowLeft" || event.key === "ArrowUp"
                  ? -1
                  : 0;
            if (!direction && event.key !== "Home" && event.key !== "End")
              return;
            event.preventDefault();
            const next =
              event.key === "Home"
                ? 0
                : event.key === "End"
                  ? options.length - 1
                  : (index + direction + options.length) % options.length;
            const nextOption = options[next];
            if (!nextOption) return;
            onChange(nextOption.value);
            const radios =
              event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>(
                '[role="radio"]',
              );
            radios?.[next]?.focus();
          }}
          className={cn(
            "min-h-control inline-flex max-w-full items-center justify-center gap-2 rounded-md px-3 py-1 type-button transition-colors",
            value === option.value
              ? "bg-card text-foreground shadow-sm"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {Icon ? <Icon aria-hidden="true" /> : null}
          <span className="min-w-0 [overflow-wrap:anywhere]">
            {option.label}
          </span>
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
    <GroupedRow>
      <div className="min-w-0 flex-1 basis-[18em]">
        <Label asChild>
          <p>{title}</p>
        </Label>
        <p className="mt-1 text-13 text-text-tertiary">{description}</p>
      </div>
      <div className="ml-auto min-w-0 max-w-full">{children}</div>
    </GroupedRow>
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
      size={7}
      spellCheck={false}
      className="w-auto min-w-0 max-w-full font-mono uppercase"
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

function SidebarWidthControl({
  sidebar,
  label,
}: {
  readonly sidebar: SidebarId;
  readonly label: string;
}) {
  const width = useThemeStore((s) => s.sidebarWidths[sidebar]);
  const setSidebarWidth = useThemeStore((s) => s.setSidebarWidth);
  return (
    <RangeControl
      label={label}
      description={
        sidebar === "dashboard"
          ? "Used when the main sidebar is expanded."
          : "The agents and chats list in the assistant."
      }
      saved={width}
      onChange={(value) => setSidebarWidth(sidebar, value)}
      {...SIDEBAR_WIDTH}
      step={4}
      defaultValue={SIDEBAR_WIDTH.default}
      formatValue={(value) => `${value}px`}
      commitOnRelease={false}
    />
  );
}

function ScaleControl({
  label,
  description,
  scale,
  onChange,
}: {
  readonly label: string;
  readonly description: string;
  readonly scale: number;
  readonly onChange: (value: number) => void;
}) {
  return (
    <RangeControl
      label={label}
      description={description}
      saved={scale}
      onChange={onChange}
      min={DISPLAY_SCALE_MIN}
      max={DISPLAY_SCALE_MAX}
      step={DISPLAY_SCALE_STEP}
      defaultValue={DISPLAY_SCALE_DEFAULT}
      formatValue={(value) => `${Number(value.toFixed(2))}×`}
    />
  );
}

function SidebarSettings() {
  const sidebarMode = useThemeStore((s) => s.sidebarMode);
  const setSidebarMode = useThemeStore((s) => s.setSidebarMode);
  return (
    <GroupedList
      title="Sidebar"
      footer="Choose how navigation opens and how much space it uses. You can also drag a sidebar’s edge to resize it."
    >
      <SettingRow
        title="Main sidebar"
        description="Expanded, icons only, or icons that expand on hover."
      >
        <SegmentedChoice
          label="Main sidebar"
          options={SIDEBAR_MODE_OPTIONS}
          value={sidebarMode}
          onChange={setSidebarMode}
        />
      </SettingRow>
      <GroupedRow>
        <SidebarWidthControl sidebar="dashboard" label="Main sidebar width" />
      </GroupedRow>
      <GroupedRow>
        <SidebarWidthControl
          sidebar="assistant"
          label="Assistant sidebar width"
        />
      </GroupedRow>
    </GroupedList>
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
    <div className="space-y-6">
      <GroupedList
        title="Colors"
        footer="Make each theme your own. Preview changes below and check that your text stays readable."
      >
        <GroupedRow>
          <SegmentedChoice
            label="Theme to edit"
            options={COLOR_MODE_OPTIONS}
            value={editMode}
            onChange={setEditMode}
          />
          {editMode !== resolved && (
            <Button variant="ghost" size="sm" onClick={() => setMode(editMode)}>
              Switch to {editMode} theme to preview
            </Button>
          )}
        </GroupedRow>
      </GroupedList>

      <section
        aria-label="Chat appearance preview"
        className="space-y-3 rounded-xl border p-4"
        style={{
          backgroundColor: color("background"),
          color: color("foreground"),
          borderColor: color("border"),
        }}
      >
        <p className="type-row-title">Chat preview · {editMode}</p>
        <UserMessageBubble
          className="ml-auto w-fit max-w-[85%] text-16"
          style={{
            backgroundColor: color("chat-bubble"),
            color: color("chat-bubble-foreground"),
          }}
        >
          Show me the services I have connected.
        </UserMessageBubble>
        <p className="text-16">
          You can manage your connected services in Plugins.
        </p>
      </section>

      <GroupedList
        title={`${editMode === "light" ? "Light" : "Dark"} theme colors`}
      >
        <ul aria-label={`${editMode} theme colors`}>
          {EDITABLE_COLORS.map(({ key, label }) => {
            const value = color(key);
            const customized = key in overrides;
            return (
              <GroupedRow asChild key={key}>
                <li>
                  <Label asChild>
                    <span className="min-w-0 flex-1">
                      {label}
                      {customized && (
                        <FieldHelp className="ml-2 text-text-tertiary">
                          Customized
                        </FieldHelp>
                      )}
                    </span>
                  </Label>
                  <input
                    type="color"
                    aria-label={`${label} color`}
                    value={value.toLowerCase()}
                    onChange={(e) =>
                      setCustomColor(editMode, key, e.target.value)
                    }
                    className="h-7 w-9 cursor-pointer rounded-md border-0 bg-transparent p-0.5"
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
              </GroupedRow>
            );
          })}
        </ul>
      </GroupedList>

      <GroupedList title="Readability">
        <ul aria-label="Contrast checks">
          {CONTRAST_CHECKS.map((check) => {
            const ratio = contrastRatio(color(check.fg), color(check.bg));
            const ok = ratio >= check.target;
            return (
              <GroupedRow asChild key={check.label}>
                <li>
                  {ok ? (
                    <Check
                      data-ui-icon=""
                      className="size-3.5 shrink-0 text-success"
                      aria-hidden
                    />
                  ) : (
                    <TriangleAlert
                      data-ui-icon=""
                      className="size-3.5 shrink-0 text-warning"
                      aria-hidden
                    />
                  )}
                  <span className="flex-1 text-muted-foreground">
                    {check.label}
                  </span>
                  <span
                    className={cn(
                      "font-mono text-13",
                      ok ? "text-muted-foreground" : "text-warning",
                    )}
                  >
                    {ratio.toFixed(1)}:1 {ok ? "" : `(needs ${check.target}:1)`}
                  </span>
                </li>
              </GroupedRow>
            );
          })}
        </ul>
      </GroupedList>

      <GroupedList>
        <GroupedRow className="justify-end">
          <Button
            variant="ghost"
            size="sm"
            disabled={customizedCount === 0}
            onClick={() => resetCustomColors(editMode)}
          >
            <RotateCcw className="size-3" />
            Reset {editMode} colors
          </Button>
        </GroupedRow>
      </GroupedList>
    </div>
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
      <GroupedList
        title="Display"
        footer="Make NyxID comfortable to read. Your choices are saved in this browser."
      >
        <SettingRow
          title="Theme"
          description="System follows your device's light or dark setting."
        >
          <SegmentedChoice
            label="Theme"
            options={THEME_OPTIONS}
            value={mode}
            onChange={setMode}
          />
        </SettingRow>
        <GroupedRow>
          <ScaleControl
            label="Text size"
            description="Resize text and icons together. Controls adapt to keep them readable."
            scale={textScale}
            onChange={setTextScale}
          />
        </GroupedRow>
        <GroupedRow>
          <ScaleControl
            label="Spacing"
            description="Choose how much room sits between controls and content."
            scale={density}
            onChange={setDensity}
          />
        </GroupedRow>
        <SettingRow
          title="Motion"
          description="Follow your device preference or use fewer animations."
        >
          <SegmentedChoice
            label="Motion"
            options={MOTION_OPTIONS}
            value={motion}
            onChange={setMotion}
          />
        </SettingRow>
        <GroupedRow className="justify-end">
          <Button variant="ghost" size="sm" onClick={resetDisplay}>
            <RotateCcw className="size-3" />
            Reset display settings
          </Button>
        </GroupedRow>
      </GroupedList>
      <GroupedList
        title="Chat layout"
        footer="Set a comfortable reading width and writing area for all your chats."
      >
        <GroupedRow>
          <ChatWidthControl />
        </GroupedRow>
        <GroupedRow>
          <ComposerHeightControl />
        </GroupedRow>
      </GroupedList>
      <SidebarSettings />
      <ColorEditor />
    </div>
  );
}

import {
  useId,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { translateUi } from "@/i18n/use-ui-translation";
import { cn } from "@/lib/utils";
import {
  AVATAR_CATALOGUE,
  avatarChoiceLabel,
  avatarChoices,
  chooseAvatar,
  isValidAvatar,
  reactionFor,
  reactionLabel,
  type AvatarConfig,
  type AvatarField,
  type AvatarMood,
  type AvatarTheme,
} from "@/lib/assistant/agent-avatar-model";
import { FACE_GEOMETRY } from "../../../../shared/agent-avatars/draw";
import { incompatibilityMessage } from "../../../../shared/agent-avatars/editor";
import { supportsNaturalEyes } from "../../../../shared/agent-avatars/emotes";
import { AvatarStill } from "./nyxbot-agent-avatar";

function selectedChoice(avatar: AvatarConfig, field: AvatarField): string {
  return (
    avatar.choices[field] ??
    (AVATAR_CATALOGUE.default_reactions as Record<string, string>)[
      field.slice(6)
    ] ??
    ""
  );
}
export function ChoiceSelect({
  label,
  value,
  values,
  optionLabel,
  onChange,
  disabledValue,
  description,
}: {
  label: string;
  value: string;
  values: readonly string[];
  optionLabel: (value: string) => string;
  onChange: (value: string) => void;
  disabledValue?: (value: string) => boolean;
  description?: string;
}) {
  const id = useId();
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      <NativeSelect
        id={id}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        aria-describedby={description ? `${id}-help` : undefined}
      >
        {values.map((choice) => (
          <option
            key={choice}
            value={choice}
            disabled={disabledValue?.(choice)}
          >
            {optionLabel(choice)}
          </option>
        ))}
      </NativeSelect>
      {description ? (
        <p id={`${id}-help`} className="type-secondary">
          {description}
        </p>
      ) : null}
    </div>
  );
}
export function ChoiceGroup({
  field,
  label,
  avatar,
  kind,
  theme,
  onChoose,
}: {
  field: AvatarField;
  label: string;
  avatar: AvatarConfig;
  kind: string;
  theme: AvatarTheme;
  onChoose: (id: string) => void;
}) {
  const selected = selectedChoice(avatar, field),
    all = avatarChoices(field, kind);
  const blocked = (id: string) =>
    !isValidAvatar(chooseAvatar(avatar, field, id));
  const reason = all
    .map((id) => incompatibilityMessage(avatar, field, id))
    .find(Boolean);
  if (field === "palette")
    return (
      <PaletteChoices
        label={label}
        avatar={avatar}
        kind={kind}
        onChoose={onChoose}
      />
    );
  if (field === "surface")
    return (
      <TextureChoices
        label={label}
        avatar={avatar}
        kind={kind}
        onChoose={onChoose}
      />
    );
  if (["head", "eyes", "motion", "frame", "ripple_palette"].includes(field)) {
    return (
      <div className="space-y-2">
        <p className="type-field-label">{label}</p>
        <RadioRow
          label={label}
          values={all}
          selected={selected}
          onSelect={onChoose}
          optionLabel={(id) => translateUi(avatarChoiceLabel(field, id))}
          disabledValue={blocked}
          className={
            field === "ripple_palette" ? "grid grid-cols-6 gap-2" : undefined
          }
          render={(id, checked) => (
            <span
              className={cn(
                "flex min-h-9 flex-col items-center justify-center gap-1 rounded-lg border border-hairline px-2 py-1.5 type-group-label",
                checked && "border-input-focus bg-overlay-strong",
              )}
            >
              {field === "ripple_palette" ? (
                <span
                  className="flex h-6 w-full min-w-7 items-center justify-center gap-0.5 rounded-md"
                  style={{ background: FACE_GEOMETRY.ripples[id]!.bg }}
                >
                  {[0, 1, 2, 3].map((i) => (
                    <span
                      key={i}
                      className="size-1 rounded-full"
                      style={{
                        background:
                          FACE_GEOMETRY.ripples[id]![i % 2 ? "a" : "b"],
                      }}
                    />
                  ))}
                </span>
              ) : field === "head" || field === "eyes" ? (
                <AvatarStill
                  config={{
                    ...avatar,
                    choices: {
                      ...avatar.choices,
                      [field]: id,
                      surface: "flat",
                      treatment: "tint",
                      background: "transparent",
                      frame: "none",
                    },
                  }}
                  kind={kind}
                  theme={theme}
                  pixels={112}
                  className="size-7"
                />
              ) : null}
              {field !== "ripple_palette"
                ? translateUi(avatarChoiceLabel(field, id))
                : null}
            </span>
          )}
        />
        {field === "ripple_palette" ? (
          <p className="type-secondary">
            {translateUi(avatarChoiceLabel(field, selected))}
          </p>
        ) : null}
        {field === "motion" ? (
          <p className="type-secondary">
            {translateUi(
              "Automatic follows activity. Quiet keeps motion subtle. Still turns it off.",
            )}
          </p>
        ) : null}
      </div>
    );
  }
  const reaction = field.startsWith("react_")
    ? reactionFor(avatar, field.slice(6) as AvatarMood, 44, true)
    : undefined;
  const naturalDefault = field.startsWith("react_")
    ? AVATAR_CATALOGUE.default_reactions[
        field.slice(6) as keyof typeof AVATAR_CATALOGUE.default_reactions
      ]
    : undefined;
  return (
    <ChoiceSelect
      label={label}
      value={selected}
      values={all}
      optionLabel={(id) =>
        translateUi(
          id === naturalDefault ? "Natural" : avatarChoiceLabel(field, id),
        )
      }
      onChange={onChoose}
      disabledValue={blocked}
      description={
        reason
          ? translateUi(reason)
          : selected === naturalDefault
            ? translateUi(
                supportsNaturalEyes(avatar.choices.surface ?? "flat")
                  ? "Animated props and eyes follow this activity."
                  : "This texture keeps the eyes still to preserve its pattern. Voice ripples remain available.",
              )
            : reaction && reaction.effective !== reaction.id
              ? `${translateUi("Compact previews")}: ${translateUi(reactionLabel(reaction.effective))}`
              : undefined
      }
    />
  );
}
function PaletteChoices({
  label,
  avatar,
  kind,
  onChoose,
}: {
  label: string;
  avatar: AvatarConfig;
  kind: string;
  onChoose: (id: string) => void;
}) {
  const all = avatarChoices("palette", kind),
    selected = avatar.choices.palette!;
  const entries = AVATAR_CATALOGUE.fields.palette as Record<
    string,
    { label: string; group: string }
  >;
  const groups = [...new Set(all.map((id) => entries[id]!.group))];
  const [browsing, setBrowsing] = useState<{
    selected: string;
    group: string;
  }>();
  const group =
    browsing?.selected === selected ? browsing.group : entries[selected]!.group;
  const values = all.filter((id) => entries[id]!.group === group);
  const id = useId();
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-3">
        <Label htmlFor={id}>{label}</Label>
        <NativeSelect
          id={id}
          aria-label={translateUi("Colour palette")}
          value={group}
          onChange={(event) =>
            setBrowsing({ selected, group: event.target.value })
          }
          className="min-h-8 py-1"
        >
          {groups.map((name) => (
            <option key={name} value={name}>
              {translateUi(name)}
            </option>
          ))}
        </NativeSelect>
      </div>
      <RadioRow
        label={label}
        values={values}
        selected={selected}
        onSelect={onChoose}
        optionLabel={(value) =>
          translateUi(avatarChoiceLabel("palette", value))
        }
        className="grid grid-cols-7 gap-2"
        render={(value, checked) => {
          const colors = FACE_GEOMETRY.palettes[value]!.colors;
          return (
            <span
              className={cn(
                "flex h-10 items-center justify-center rounded-full p-1",
                checked &&
                  "ring-2 ring-input-focus ring-offset-2 ring-offset-card",
              )}
            >
              <span
                className="block size-8 rounded-full border border-hairline"
                style={{
                  background: `linear-gradient(135deg,${colors[0]},${colors[1]})`,
                }}
              />
            </span>
          );
        }}
      />
      <p className="type-secondary">
        {translateUi(avatarChoiceLabel("palette", selected))}
      </p>
    </div>
  );
}
function TextureChoices({
  label,
  avatar,
  kind,
  onChoose,
}: {
  label: string;
  avatar: AvatarConfig;
  kind: string;
  onChoose: (id: string) => void;
}) {
  const helpId = useId();
  const all = avatarChoices("surface", kind),
    selected = avatar.choices.surface!;
  const family = selected.startsWith("dither-")
    ? "dither"
    : selected.startsWith("ascii-")
      ? "ascii"
      : "flat";
  const values = all.filter((id) =>
    family === "flat" ? id === "flat" : id.startsWith(`${family}-`),
  );
  const optionsFor = (value: string) =>
    all.filter((id) =>
      value === "flat" ? id === "flat" : id.startsWith(`${value}-`),
    );
  const valid = (id: string) =>
    isValidAvatar(chooseAvatar(avatar, "surface", id));
  const blocked = all.some((id) => !valid(id));
  return (
    <div className="space-y-2">
      <p className="type-field-label">{label}</p>
      <RadioRow
        label={label}
        values={["flat", "dither", "ascii"]}
        descriptionId={blocked ? helpId : undefined}
        selected={family}
        optionLabel={(value) =>
          translateUi(
            value === "flat" ? "Flat" : value === "dither" ? "Dither" : "ASCII",
          )
        }
        disabledValue={(value) => !optionsFor(value).some(valid)}
        onSelect={(value) => {
          const next = optionsFor(value).find(valid);
          if (next) onChoose(next);
        }}
        render={(value, checked) => (
          <span
            className={cn(
              "block rounded-lg border border-hairline px-3 py-2 type-group-label",
              checked && "border-input-focus bg-overlay-strong",
            )}
          >
            {translateUi(
              value === "flat"
                ? "Flat"
                : value === "dither"
                  ? "Dither"
                  : "ASCII",
            )}
          </span>
        )}
      />
      {family !== "flat" ? (
        <ChoiceSelect
          label={translateUi("Texture style")}
          value={selected}
          values={values}
          optionLabel={(value) =>
            translateUi(avatarChoiceLabel("surface", value))
          }
          disabledValue={(value) => !valid(value)}
          onChange={onChoose}
        />
      ) : null}
      {blocked ? (
        <p id={helpId} className="type-secondary">
          {translateUi(
            "Textured styles need a filled face. Choose another colour treatment before selecting a texture.",
          )}
        </p>
      ) : null}
    </div>
  );
}

/** A keyboard-navigable radio row (roving tab stop, arrows, Home/End). */
function RadioRow({
  label,
  values,
  selected,
  onSelect,
  render,
  optionLabel,
  className,
  disabledValue,
  descriptionId,
}: {
  readonly label: string;
  readonly values: readonly string[];
  readonly selected: string;
  readonly onSelect: (value: string) => void;
  readonly render: (value: string, checked: boolean) => ReactNode;
  readonly optionLabel?: (value: string) => string;
  readonly className?: string;
  readonly disabledValue?: (value: string) => boolean;
  readonly descriptionId?: string;
}) {
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  const move = (event: KeyboardEvent, index: number) => {
    const available = values
      .map((value, i) => (disabledValue?.(value) ? -1 : i))
      .filter((i) => i >= 0);
    const position = available.indexOf(index),
      last = available.length - 1;
    const step =
      event.key === "ArrowRight" || event.key === "ArrowDown"
        ? 1
        : event.key === "ArrowLeft" || event.key === "ArrowUp"
          ? -1
          : 0;
    const next =
      event.key === "Home"
        ? available[0]
        : event.key === "End"
          ? available[last]
          : step
            ? available[(position + step + available.length) % available.length]
            : undefined;
    if (next === undefined) return;
    event.preventDefault();
    onSelect(values[next]!);
    buttons.current[next]?.focus();
  };
  const active = Math.max(0, values.indexOf(selected));
  return (
    <div
      role="radiogroup"
      aria-label={label}
      aria-describedby={descriptionId}
      className={cn("flex flex-wrap gap-1.5", className)}
    >
      {values.map((value, index) => {
        const checked = value === selected;
        return (
          <button
            key={value}
            ref={(element) => {
              buttons.current[index] = element;
            }}
            type="button"
            role="radio"
            aria-checked={checked}
            disabled={disabledValue?.(value)}
            aria-describedby={
              disabledValue?.(value) ? descriptionId : undefined
            }
            aria-label={optionLabel?.(value)}
            title={optionLabel?.(value)}
            tabIndex={index === active ? 0 : -1}
            onClick={() => onSelect(value)}
            onKeyDown={(event) => move(event, index)}
            className="rounded-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-input-focus"
          >
            {render(value, checked)}
          </button>
        );
      })}
    </div>
  );
}

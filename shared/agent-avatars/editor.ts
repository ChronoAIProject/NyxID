import {
  AVATAR_MOODS,
  avatarMoodLabel,
  isValidAvatar,
  chooseAvatar,
  type AvatarConfig,
  type AvatarField,
} from "./model";

export function incompatibilityMessage(
  config: AvatarConfig,
  field: AvatarField,
  id: string,
): string | undefined {
  if (isValidAvatar(chooseAvatar(config, field, id))) return undefined;
  return field === "surface"
    ? "Textured styles need a filled face. Choose another colour treatment before selecting a texture."
    : "This outline needs the Flat rendering style. Choose Flat first.";
}
export const EDITOR_SLOTS: readonly {
  field: AvatarField;
  label: string;
  group: string;
}[] = [
  { field: "head", label: "Head", group: "Face" },
  { field: "eyes", label: "Resting eyes", group: "Face" },
  { field: "palette", label: "Palette", group: "Colour" },
  { field: "treatment", label: "Colour treatment", group: "Colour" },
  { field: "background", label: "Background", group: "Background" },
  { field: "ripple_palette", label: "Ripple palette", group: "Background" },
  { field: "frame", label: "Framing", group: "Background" },
  { field: "surface", label: "Rendering style", group: "Style" },
  { field: "motion", label: "Reactions", group: "Reactions" },
  ...AVATAR_MOODS.map((state) => ({
    field: `react_${state}` as AvatarField,
    label: avatarMoodLabel(state),
    group: "Reactions",
  })),
];
export const APPEARANCE_COPY = [
  "Face",
  "Colour",
  "Background",
  "Style",
  "Reactions",
  "Head",
  "Resting eyes",
  "Palette",
  "Colour treatment",
  "Ripple palette",
  "Framing",
  "Rendering style",
  "NyxBot starts with a round face in brand purple. Every appearance choice can be changed.",
  "Treatment background",
  "Show full catalogue",
  "Show curated choices",
  "Adopt face kit",
  "This appearance uses a different version. Adopt the face kit or reset explicitly to change it.",
  "Large preview",
  "Compact previews",
  "Export saved avatar",
  "Transparent background",
  "Save changes before exporting.",
  "Native textures are static. Reactions use quiet motion.",
  "Preview states do not change live activity.",
  "This outline needs the Flat rendering style. Choose Flat first.",
  "Textured styles need a filled face. Choose another colour treatment before selecting a texture.",
  "A crisp face preserves these eyes at this texture's grid size.",
  "Uses the selected colour treatment unless you choose another background.",
] as const;

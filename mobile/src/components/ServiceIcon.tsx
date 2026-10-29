import { useCallback, useState, type ReactElement } from "react";
import { Image, View } from "react-native";
import { SvgUri } from "react-native-svg";
import { useTheme } from "../theme/ThemeContext";
import { FallbackIcon, SERVICE_ICONS } from "./service-icons";

export const SERVICE_ICON_SIZES = {
  "2xs": 14,
  xs: 16,
  sm: 20,
  md: 24,
  lg: 32,
  xl: 36,
} as const;
export type ServiceIconSize = keyof typeof SERVICE_ICON_SIZES;

/** Public images use the same URL policy as the web, never authenticated API calls. */
export function serviceIconSource(value?: string | null) {
  if (!value || value.length > 2048 || value.includes("#")) return null;
  try {
    const url = new URL(value);
    if (
      !["http:", "https:"].includes(url.protocol) ||
      !url.hostname ||
      url.username ||
      url.password
    )
      return null;
    return {
      uri: url.href,
      svg:
        /\/catalog-icons\/[^/]+\/?$/i.test(url.pathname) ||
        /\.svg$/i.test(url.pathname),
    };
  } catch {
    return null;
  }
}

export function ServiceIcon({
  slug,
  iconUrl,
  size = "sm",
  color,
}: {
  readonly slug?: string | null;
  readonly iconUrl?: string | null;
  readonly size?: ServiceIconSize;
  readonly color?: string;
}) {
  const { colors } = useTheme();
  const source = serviceIconSource(iconUrl);
  if (!slug && !iconUrl) return null;
  const pixels = SERVICE_ICON_SIZES[size];
  const tint = color ?? colors.textSecondary;
  const Glyph =
    slug && Object.prototype.hasOwnProperty.call(SERVICE_ICONS, slug)
      ? SERVICE_ICONS[slug]!
      : FallbackIcon;
  const fallback = <Glyph size={pixels} color={tint} />;
  return (
    <View
      accessible={false}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={{
        width: pixels,
        height: pixels,
        flexShrink: 0,
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      {source ? (
        <RemoteIcon
          key={source.uri}
          source={source}
          size={pixels}
          color={tint}
          fallback={fallback}
        />
      ) : (
        fallback
      )}
    </View>
  );
}

function RemoteIcon({
  source,
  size,
  color,
  fallback,
}: {
  source: NonNullable<ReturnType<typeof serviceIconSource>>;
  size: number;
  color: string;
  fallback: ReactElement;
}) {
  const [mode, setMode] = useState(source.svg ? "svg" : "image");
  const onSvgError = useCallback(() => setMode("failed"), []);
  const onImageError = useCallback(() => setMode("svg"), []);
  if (mode === "failed") return fallback;
  if (mode === "svg")
    return (
      <SvgUri
        uri={source.uri}
        width={size}
        height={size}
        color={color}
        fallback={fallback}
        onError={onSvgError}
      />
    );
  return (
    <Image
      accessible={false}
      source={{ uri: source.uri }}
      resizeMode="contain"
      style={{ width: size, height: size }}
      // Extensionless external SVGs cannot be decoded by Image. Try the SVG
      // decoder before falling back to the registry glyph.
      onError={onImageError}
    />
  );
}

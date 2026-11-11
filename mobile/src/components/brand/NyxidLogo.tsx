import { View } from "react-native";
import { SvgXml } from "react-native-svg";
import { useTheme } from "../../theme/ThemeContext";
import { nyxidLogoDarkXml, nyxidLogoXml } from "./brandVectors";

/** The web sidebar's h-5 mark and wordmark, with the resolved theme variant. */
export function NyxidLogo({ height = 20 }: { height?: number }) {
  const { mode } = useTheme();
  const width = (height * 1062) / 301;
  return (
    <View
      accessible
      accessibilityRole="image"
      accessibilityLabel="NyxID"
      style={{ width, height }}
    >
      <SvgXml
        xml={mode === "light" ? nyxidLogoDarkXml : nyxidLogoXml}
        width={width}
        height={height}
        accessible={false}
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
      />
    </View>
  );
}

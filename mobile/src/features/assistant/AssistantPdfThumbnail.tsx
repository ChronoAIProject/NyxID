import { requireNativeView, requireOptionalNativeModule } from "expo";
import type { ViewProps } from "react-native";

type Props = ViewProps & {
  uri: string;
  onPreviewError: () => void;
};

// Older development binaries can still compose documents before being rebuilt.
export const AssistantPdfThumbnail = requireOptionalNativeModule(
  "NyxPdfThumbnail",
)
  ? requireNativeView<Props>("NyxPdfThumbnail")
  : null;

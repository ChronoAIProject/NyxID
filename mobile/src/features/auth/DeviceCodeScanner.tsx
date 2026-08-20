import { useMemo, useRef, useState } from "react";
import {
  ActivityIndicator,
  Linking,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import {
  CameraView,
  useCameraPermissions,
  type BarcodeScanningResult,
} from "expo-camera";
import { Camera, X } from "lucide-react-native";

import { PrimaryButton } from "../../components/PrimaryButton";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import { radius, spacing, typeScale, TOUCH_TARGET } from "../../theme/designTokens";
import { extractAuthDeviceUserCodeFromQr } from "./deviceUserCode";

type DeviceCodeScannerProps = {
  onCancel: () => void;
  onCode: (userCode: string) => void;
};

export function DeviceCodeScanner({ onCancel, onCode }: DeviceCodeScannerProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const [permission, requestPermission] = useCameraPermissions();
  const [scanError, setScanError] = useState<string | null>(null);
  const [permissionPending, setPermissionPending] = useState(false);
  const scanHandled = useRef(false);

  const handleBarcode = (result: BarcodeScanningResult) => {
    if (scanHandled.current) return;
    scanHandled.current = true;

    const userCode = extractAuthDeviceUserCodeFromQr(result.data);
    if (!userCode) {
      setScanError("This QR code is not a valid NyxID login request.");
      return;
    }

    onCode(userCode);
  };

  const retryScan = () => {
    scanHandled.current = false;
    setScanError(null);
  };

  const handlePermissionRequest = async () => {
    setPermissionPending(true);
    try {
      await requestPermission();
    } finally {
      setPermissionPending(false);
    }
  };

  if (!permission) {
    return (
      <View style={styles.centered}>
        <ActivityIndicator color={colors.primary} />
      </View>
    );
  }

  if (!permission.granted) {
    return (
      <View style={styles.permissionScreen}>
        <View style={styles.permissionIcon}>
          <Camera size={28} color={colors.primary} />
        </View>
        <Text style={styles.permissionTitle}>Camera access needed</Text>
        <Text style={styles.permissionBody}>
          NyxID uses the camera only to read login approval QR codes.
        </Text>
        {permission.canAskAgain ? (
          <PrimaryButton
            label={permissionPending ? "Requesting access..." : "Allow camera"}
            disabled={permissionPending}
            onPress={() => void handlePermissionRequest()}
          />
        ) : (
          <>
            <Text style={styles.permissionError}>
              Enable camera access for NyxID in system settings, then try again.
            </Text>
            <PrimaryButton label="Open settings" onPress={() => void Linking.openSettings()} />
          </>
        )}
        <PrimaryButton label="Back" kind="ghost" disabled={permissionPending} onPress={onCancel} />
      </View>
    );
  }

  return (
    <View style={styles.cameraScreen}>
      <CameraView
        style={StyleSheet.absoluteFill}
        facing="back"
        barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
        onBarcodeScanned={scanHandled.current ? undefined : handleBarcode}
      />
      <View style={styles.cameraShade} pointerEvents="none" />
      <View style={styles.cameraHeader}>
        <Text style={styles.cameraTitle}>Scan login QR code</Text>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Close scanner"
          onPress={onCancel}
          style={styles.closeButton}
        >
          <X size={22} color="#FFFFFF" />
        </Pressable>
      </View>
      <View style={styles.scanArea} pointerEvents="none" />
      <View style={styles.cameraFooter}>
        {scanError ? (
          <View style={styles.scanErrorPanel}>
            <Text style={styles.scanErrorText}>{scanError}</Text>
            <PrimaryButton label="Scan another code" kind="ghost" onPress={retryScan} />
          </View>
        ) : (
          <Text style={styles.cameraHint}>Center the complete QR code inside the frame.</Text>
        )}
      </View>
    </View>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    centered: {
      flex: 1,
      alignItems: "center",
      justifyContent: "center",
      backgroundColor: c.bg,
    },
    permissionScreen: {
      flex: 1,
      justifyContent: "center",
      padding: spacing.huge,
      gap: spacing.lg,
      backgroundColor: c.bg,
    },
    permissionIcon: {
      width: 56,
      height: 56,
      borderRadius: radius.lg,
      alignItems: "center",
      justifyContent: "center",
      backgroundColor: c.primaryTone.bg,
      borderWidth: 1,
      borderColor: c.primaryTone.border,
    },
    permissionTitle: { ...typeScale.h2, color: c.textPrimary },
    permissionBody: { ...typeScale.description, color: c.textSecondary },
    permissionError: { ...typeScale.body, color: c.danger },
    cameraScreen: { flex: 1, backgroundColor: "#000000" },
    cameraShade: { ...StyleSheet.absoluteFillObject, backgroundColor: "rgba(0,0,0,0.22)" },
    cameraHeader: {
      position: "absolute",
      top: 0,
      left: 0,
      right: 0,
      paddingTop: 54,
      paddingHorizontal: spacing.xxl,
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "space-between",
    },
    cameraTitle: { ...typeScale.h2, color: "#FFFFFF" },
    closeButton: {
      width: TOUCH_TARGET,
      height: TOUCH_TARGET,
      borderRadius: radius.full,
      alignItems: "center",
      justifyContent: "center",
      backgroundColor: "rgba(0,0,0,0.55)",
      borderWidth: 1,
      borderColor: "rgba(255,255,255,0.28)",
    },
    scanArea: {
      position: "absolute",
      width: 252,
      height: 252,
      top: "50%",
      left: "50%",
      marginTop: -144,
      marginLeft: -126,
      borderRadius: radius.lg,
      borderWidth: 3,
      borderColor: "#FFFFFF",
      backgroundColor: "rgba(255,255,255,0.03)",
    },
    cameraFooter: {
      position: "absolute",
      left: spacing.xxl,
      right: spacing.xxl,
      bottom: 52,
      alignItems: "center",
    },
    cameraHint: {
      ...typeScale.bodyStrong,
      color: "#FFFFFF",
      textAlign: "center",
      backgroundColor: "rgba(0,0,0,0.62)",
      paddingHorizontal: spacing.xxl,
      paddingVertical: spacing.md,
      borderRadius: radius.md,
    },
    scanErrorPanel: {
      width: "100%",
      gap: spacing.lg,
      padding: spacing.xxl,
      borderRadius: radius.lg,
      backgroundColor: c.card,
      borderWidth: 1,
      borderColor: c.dangerTone.border,
    },
    scanErrorText: { ...typeScale.body, color: c.danger, textAlign: "center" },
  });

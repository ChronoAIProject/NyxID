import { useEffect, useMemo, useRef, useState } from "react";
import {
  ActivityIndicator,
  AppState,
  Linking,
  Pressable,
  StyleSheet,
  Text,
  useWindowDimensions,
  View,
} from "react-native";
import {
  CameraView,
  useCameraPermissions,
  type BarcodeScanningResult,
} from "expo-camera";
import { StatusBar } from "expo-status-bar";
import { CameraOff, ChevronLeft } from "lucide-react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { PrimaryButton } from "../../components/PrimaryButton";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import {
  radius,
  spacing,
  typeScale,
  TOUCH_TARGET,
} from "../../theme/designTokens";
import { extractAuthDeviceUserCodeFromQr } from "./deviceUserCode";

type DeviceCodeScannerProps = {
  onCancel: () => void;
  onCode: (userCode: string) => void;
  onManualEntry: () => void;
  paused?: boolean;
};

const INVALID_QR_REARM_DELAY_MS = 4_000;

export function DeviceCodeScanner({
  onCancel,
  onCode,
  onManualEntry,
  paused = false,
}: DeviceCodeScannerProps) {
  const { colors, mode } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const insets = useSafeAreaInsets();
  const { width, height } = useWindowDimensions();
  const [permission, requestPermission, getPermission] = useCameraPermissions();
  const [scanError, setScanError] = useState<string | null>(null);
  const [permissionError, setPermissionError] = useState<string | null>(null);
  const [permissionPending, setPermissionPending] = useState(false);
  const scanHandled = useRef(false);
  const reticleSize = Math.max(196, Math.min(272, width - 64, height * 0.35));

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (nextState) => {
      if (nextState !== "active") return;
      void getPermission().catch(() => {
        setPermissionError(
          "Camera access could not be checked. Please try again.",
        );
      });
    });
    return () => subscription.remove();
  }, [getPermission]);

  useEffect(() => {
    if (!scanError || paused) return;

    const timeout = setTimeout(() => {
      scanHandled.current = false;
      setScanError(null);
    }, INVALID_QR_REARM_DELAY_MS);

    return () => clearTimeout(timeout);
  }, [paused, scanError]);

  const handleBarcode = (result: BarcodeScanningResult) => {
    if (paused || scanHandled.current) return;
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
    setPermissionError(null);
    try {
      await requestPermission();
    } catch {
      setPermissionError(
        "Camera access could not be requested. Try again or enter the code manually.",
      );
    } finally {
      setPermissionPending(false);
    }
  };

  const handleOpenSettings = async () => {
    setPermissionPending(true);
    setPermissionError(null);
    try {
      await Linking.openSettings();
    } catch {
      setPermissionError(
        "Settings could not be opened. Enable Camera for NyxID in system settings.",
      );
    } finally {
      setPermissionPending(false);
    }
  };

  const header = (camera: boolean) => (
    <View
      style={[
        styles.header,
        camera ? styles.cameraHeader : styles.pageHeader,
        { height: insets.top + 52, paddingTop: insets.top },
      ]}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Back"
        hitSlop={4}
        onPress={onCancel}
        style={({ pressed }) => [
          styles.backButton,
          camera ? styles.cameraBackButton : styles.pageBackButton,
          pressed && styles.pressed,
        ]}
      >
        <ChevronLeft
          size={24}
          color={camera ? "#FFFFFF" : colors.textPrimary}
        />
      </Pressable>
      <Text style={[styles.headerTitle, camera && styles.cameraHeaderTitle]}>
        Scan login code
      </Text>
      <View style={styles.headerSpacer} />
    </View>
  );

  if (!permission) {
    return (
      <View style={styles.permissionScreen}>
        <StatusBar style={mode === "dark" ? "light" : "dark"} />
        {header(false)}
        <View style={styles.loadingBody}>
          <ActivityIndicator color={colors.primary} />
        </View>
      </View>
    );
  }

  if (!permission.granted) {
    const canRequest = permission.canAskAgain;
    return (
      <View style={styles.permissionScreen}>
        <StatusBar style={mode === "dark" ? "light" : "dark"} />
        {header(false)}
        <View
          style={[
            styles.permissionContent,
            {
              paddingBottom:
                Math.max(insets.bottom, spacing.xxl) + spacing.huge,
            },
          ]}
        >
          <View style={styles.permissionCopy}>
            <View style={styles.permissionIcon}>
              <CameraOff size={22} color={colors.textSecondary} />
            </View>
            <Text style={styles.permissionTitle}>Camera access is off</Text>
            <Text style={styles.permissionBody}>
              {canRequest
                ? "Enable camera access to scan a NyxID login QR code. NyxID does not record audio."
                : "Camera access is disabled for NyxID. Open camera settings to enable scanning."}
            </Text>
            {permissionError ? (
              <Text style={styles.permissionError}>{permissionError}</Text>
            ) : null}
          </View>
          <View style={styles.permissionActions}>
            <PrimaryButton
              label={
                canRequest
                  ? permissionPending
                    ? "Requesting access..."
                    : "Enable camera"
                  : permissionPending
                    ? "Opening settings..."
                    : "Open camera settings"
              }
              disabled={permissionPending}
              onPress={() =>
                void (canRequest
                  ? handlePermissionRequest()
                  : handleOpenSettings())
              }
            />
            <View style={styles.permissionDivider}>
              <View style={styles.permissionDividerLine} />
              <Text style={styles.permissionDividerText}>or</Text>
              <View style={styles.permissionDividerLine} />
            </View>
            <PrimaryButton
              label="Manually enter code"
              kind="ghost"
              disabled={permissionPending}
              onPress={onManualEntry}
            />
          </View>
        </View>
      </View>
    );
  }

  return (
    <View style={styles.cameraScreen}>
      <StatusBar style="light" />
      <CameraView
        style={StyleSheet.absoluteFill}
        facing="back"
        barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
        onBarcodeScanned={
          paused || scanHandled.current ? undefined : handleBarcode
        }
      />
      <View style={styles.cameraShade} pointerEvents="none" />
      {header(true)}
      <View style={styles.scannerGuide} pointerEvents="none">
        <Text style={styles.cameraInstruction}>
          Center the complete QR code inside the frame
        </Text>
        <View
          style={[styles.scanArea, { width: reticleSize, height: reticleSize }]}
        >
          <View style={[styles.reticleCorner, styles.cornerTopLeft]} />
          <View style={[styles.reticleCorner, styles.cornerTopRight]} />
          <View style={[styles.reticleCorner, styles.cornerBottomLeft]} />
          <View style={[styles.reticleCorner, styles.cornerBottomRight]} />
        </View>
      </View>
      <View
        style={[
          styles.cameraFooter,
          { bottom: Math.max(insets.bottom, spacing.xxl) + spacing.lg },
        ]}
      >
        {scanError ? (
          <View style={styles.scanErrorPanel}>
            <Text accessibilityRole="alert" style={styles.scanErrorText}>
              {scanError}
            </Text>
            <Pressable
              accessibilityRole="button"
              onPress={retryScan}
              style={({ pressed }) => [
                styles.retryButton,
                pressed && styles.pressed,
              ]}
            >
              <Text style={styles.retryButtonText}>Scan another code</Text>
            </Pressable>
          </View>
        ) : null}
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Manually enter login code"
          onPress={onManualEntry}
          style={({ pressed }) => [
            styles.manualButton,
            pressed && styles.cameraButtonPressed,
          ]}
        >
          <Text style={styles.manualButtonText}>Manually enter code</Text>
        </Pressable>
      </View>
    </View>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    loadingBody: {
      flex: 1,
      alignItems: "center",
      justifyContent: "center",
    },
    permissionScreen: {
      flex: 1,
      backgroundColor: c.bg,
    },
    header: {
      paddingHorizontal: spacing.lg,
      flexDirection: "row",
      alignItems: "center",
    },
    pageHeader: {
      borderBottomWidth: StyleSheet.hairlineWidth,
      borderBottomColor: c.borderSoft,
      backgroundColor: c.bg,
    },
    cameraHeader: {
      position: "absolute",
      zIndex: 2,
      top: 0,
      left: 0,
      right: 0,
      backgroundColor: "rgba(0,0,0,0.34)",
    },
    headerTitle: {
      ...typeScale.title,
      flex: 1,
      color: c.textPrimary,
      textAlign: "center",
      letterSpacing: 0,
    },
    cameraHeaderTitle: { color: "#FFFFFF" },
    headerSpacer: { width: TOUCH_TARGET, height: TOUCH_TARGET },
    backButton: {
      width: TOUCH_TARGET,
      height: TOUCH_TARGET,
      borderRadius: radius.md,
      alignItems: "center",
      justifyContent: "center",
      borderWidth: 1,
    },
    pageBackButton: {
      backgroundColor: c.ghostBg,
      borderColor: c.border,
    },
    cameraBackButton: {
      backgroundColor: "rgba(0,0,0,0.46)",
      borderColor: "rgba(255,255,255,0.28)",
    },
    permissionContent: {
      flex: 1,
      paddingHorizontal: spacing.huge,
      justifyContent: "center",
      gap: spacing.huge,
    },
    permissionCopy: {
      alignItems: "center",
      gap: spacing.lg,
    },
    permissionIcon: {
      width: 48,
      height: 48,
      borderRadius: radius.md,
      alignItems: "center",
      justifyContent: "center",
      backgroundColor: c.ghostBg,
      borderWidth: 1,
      borderColor: c.border,
      marginBottom: spacing.sm,
    },
    permissionTitle: {
      ...typeScale.h2,
      color: c.textPrimary,
      textAlign: "center",
      letterSpacing: 0,
    },
    permissionBody: {
      ...typeScale.description,
      color: c.textSecondary,
      maxWidth: 320,
      textAlign: "center",
      letterSpacing: 0,
    },
    permissionError: {
      ...typeScale.body,
      color: c.danger,
      textAlign: "center",
      letterSpacing: 0,
    },
    permissionActions: {
      width: "100%",
      maxWidth: 360,
      alignSelf: "center",
      gap: spacing.lg,
    },
    permissionDivider: {
      flexDirection: "row",
      alignItems: "center",
    },
    permissionDividerLine: {
      flex: 1,
      height: StyleSheet.hairlineWidth,
      backgroundColor: c.border,
    },
    permissionDividerText: {
      ...typeScale.small,
      marginHorizontal: spacing.sm,
      color: c.textMuted,
      letterSpacing: 0,
    },
    cameraScreen: { flex: 1, backgroundColor: "#000000" },
    cameraShade: {
      ...StyleSheet.absoluteFillObject,
      backgroundColor: "rgba(0,0,0,0.20)",
    },
    scannerGuide: {
      ...StyleSheet.absoluteFillObject,
      alignItems: "center",
      justifyContent: "center",
      gap: spacing.xxl,
      transform: [{ translateY: -28 }],
    },
    cameraInstruction: {
      ...typeScale.bodyStrong,
      color: "#FFFFFF",
      textAlign: "center",
      letterSpacing: 0,
      maxWidth: 300,
      backgroundColor: "rgba(0,0,0,0.58)",
      paddingHorizontal: spacing.xxl,
      paddingVertical: spacing.sm,
      borderRadius: radius.md,
      overflow: "hidden",
    },
    scanArea: {
      position: "relative",
    },
    reticleCorner: {
      position: "absolute",
      width: 46,
      height: 46,
      borderColor: "#FFFFFF",
      borderWidth: 0,
    },
    cornerTopLeft: {
      top: 0,
      left: 0,
      borderTopWidth: 4,
      borderLeftWidth: 4,
      borderTopLeftRadius: radius.md,
    },
    cornerTopRight: {
      top: 0,
      right: 0,
      borderTopWidth: 4,
      borderRightWidth: 4,
      borderTopRightRadius: radius.md,
    },
    cornerBottomLeft: {
      bottom: 0,
      left: 0,
      borderBottomWidth: 4,
      borderLeftWidth: 4,
      borderBottomLeftRadius: radius.md,
    },
    cornerBottomRight: {
      right: 0,
      bottom: 0,
      borderRightWidth: 4,
      borderBottomWidth: 4,
      borderBottomRightRadius: radius.md,
    },
    cameraFooter: {
      position: "absolute",
      zIndex: 2,
      left: spacing.xxl,
      right: spacing.xxl,
      alignItems: "center",
      gap: spacing.lg,
    },
    scanErrorPanel: {
      width: "100%",
      maxWidth: 420,
      gap: spacing.lg,
      padding: spacing.xxl,
      borderRadius: radius.lg,
      backgroundColor: "rgba(7,6,14,0.88)",
      borderWidth: 1,
      borderColor: "rgba(248,113,113,0.45)",
    },
    scanErrorText: {
      ...typeScale.body,
      color: "#FCA5A5",
      textAlign: "center",
      letterSpacing: 0,
    },
    retryButton: {
      minHeight: TOUCH_TARGET,
      borderRadius: radius.md,
      borderWidth: 1,
      borderColor: "rgba(255,255,255,0.30)",
      backgroundColor: "rgba(255,255,255,0.06)",
      alignItems: "center",
      justifyContent: "center",
      paddingHorizontal: spacing.xxl,
    },
    retryButtonText: { ...typeScale.label, color: "#FFFFFF", letterSpacing: 0 },
    manualButton: {
      width: "100%",
      maxWidth: 420,
      minHeight: TOUCH_TARGET,
      borderRadius: radius.md,
      borderWidth: 1,
      borderColor: "rgba(255,255,255,0.72)",
      backgroundColor: "rgba(0,0,0,0.48)",
      alignItems: "center",
      justifyContent: "center",
      paddingHorizontal: spacing.xxl,
    },
    manualButtonText: {
      ...typeScale.label,
      color: "#FFFFFF",
      letterSpacing: 0,
    },
    pressed: { opacity: 0.72 },
    cameraButtonPressed: { backgroundColor: "rgba(255,255,255,0.14)" },
  });

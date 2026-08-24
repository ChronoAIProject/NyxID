#!/usr/bin/env node

const { execFileSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const plist = require("@expo/plist").default;

const MOBILE_ROOT = path.join(__dirname, "..");
const PACKAGE_JSON_PATH = path.join(MOBILE_ROOT, "package.json");
const PODFILE_LOCK_PATH = path.join(MOBILE_ROOT, "ios", "Podfile.lock");
const INFO_PLIST_PATH = path.join(MOBILE_ROOT, "ios", "NyxIDMobile", "Info.plist");
const REQUIRED_IOS_PODS = ["ExpoCameraBarcodeScanning", "ZXingObjC"];

function runJson(command, args, env = process.env) {
  const commandEnv = { ...env };
  delete commandEnv.FORCE_COLOR;
  delete commandEnv.NO_COLOR;
  return JSON.parse(
    execFileSync(command, args, {
      cwd: MOBILE_ROOT,
      encoding: "utf8",
      env: commandEnv,
      stdio: ["ignore", "pipe", "inherit"],
    })
  );
}

function runAutolinking(command) {
  return runJson("pnpm", [
    "exec",
    "expo-modules-autolinking",
    command,
    "--platform",
    "ios",
    "--json",
  ]);
}

function podNamesFromLockfile(contents) {
  const names = new Set();
  for (const match of contents.matchAll(/^  - "?([^ (":]+).*$/gm)) {
    const name = match[1]?.split("/")[0];
    if (name) names.add(name);
  }
  return names;
}

function expectedDirectNativePods(packageJson) {
  const directDependencies = new Set(Object.keys(packageJson.dependencies ?? {}));
  const expected = new Map();

  const expoModules = runAutolinking("resolve").modules ?? [];
  for (const module of expoModules) {
    if (!directDependencies.has(module.packageName)) continue;
    const pods = (module.pods ?? []).map((pod) => pod.podName).filter(Boolean);
    if (pods.length > 0) expected.set(module.packageName, pods);
  }

  const reactNativeModules = runAutolinking("react-native-config").dependencies ?? {};
  for (const [packageName, module] of Object.entries(reactNativeModules)) {
    if (!directDependencies.has(packageName)) continue;
    const podspecPath = module.platforms?.ios?.podspecPath;
    if (!podspecPath) continue;
    expected.set(packageName, [path.basename(podspecPath, ".podspec")]);
  }

  return expected;
}

function resolvedExpoConfig() {
  const fallbackEnv = {
    APP_ENV: "dev",
    DEV_API_BASE_URL: "http://localhost:3001/api/v1",
    DEV_FRONTEND_URL: "http://localhost:3000",
    DEV_IOS_BUNDLE_ID: "dev.nyxid.nativecheck",
    DEV_ANDROID_PACKAGE: "dev.nyxid.nativecheck",
    DEV_IOS_BUILD_NUMBER: "1",
  };
  return runJson(
    "pnpm",
    ["exec", "expo", "config", "--type", "public", "--json"],
    { ...fallbackEnv, ...process.env }
  );
}

function declaredIosPermissions(config) {
  const permissions = new Map(
    Object.entries(config.ios?.infoPlist ?? {}).filter(
      ([key, value]) => key.endsWith("UsageDescription") && typeof value === "string"
    )
  );

  const cameraPlugin = (config.plugins ?? []).find(
    (plugin) => Array.isArray(plugin) && plugin[0] === "expo-camera"
  );
  const cameraOptions = Array.isArray(cameraPlugin) ? cameraPlugin[1] ?? {} : {};
  if (typeof cameraOptions.cameraPermission === "string") {
    permissions.set("NSCameraUsageDescription", cameraOptions.cameraPermission);
  }
  if (typeof cameraOptions.microphonePermission === "string") {
    permissions.set("NSMicrophoneUsageDescription", cameraOptions.microphonePermission);
  }

  return {
    permissions,
    microphoneSuppressed: cameraOptions.microphonePermission === false,
  };
}

function main() {
  const packageJson = JSON.parse(fs.readFileSync(PACKAGE_JSON_PATH, "utf8"));
  const podfileLock = fs.readFileSync(PODFILE_LOCK_PATH, "utf8");
  const lockedPods = podNamesFromLockfile(podfileLock);
  const expectedPods = expectedDirectNativePods(packageJson);
  const failures = [];

  for (const [packageName, pods] of expectedPods) {
    for (const pod of pods) {
      if (!lockedPods.has(pod)) {
        failures.push(`${packageName} is declared in package.json but pod ${pod} is missing`);
      }
    }
  }

  for (const pod of REQUIRED_IOS_PODS) {
    if (!lockedPods.has(pod)) {
      failures.push(`required iOS companion pod ${pod} is missing`);
    }
  }

  const config = resolvedExpoConfig();
  const nativeInfoPlist = plist.parse(fs.readFileSync(INFO_PLIST_PATH, "utf8"));
  const { permissions, microphoneSuppressed } = declaredIosPermissions(config);
  for (const [key, value] of permissions) {
    if (nativeInfoPlist[key] !== value) {
      failures.push(
        `${key} in Info.plist does not match app.config.ts (${JSON.stringify(value)})`
      );
    }
  }
  if (microphoneSuppressed && nativeInfoPlist.NSMicrophoneUsageDescription !== undefined) {
    failures.push(
      "NSMicrophoneUsageDescription must be absent when expo-camera microphonePermission is false"
    );
  }

  if (failures.length > 0) {
    process.stderr.write(
      `Committed iOS native state is out of sync:\n- ${failures.join("\n- ")}\n`
    );
    process.exit(1);
  }

  const nativePackages = [...expectedPods.keys()].sort();
  process.stdout.write(
    `iOS native sync verified for ${nativePackages.length} direct native dependencies, ${REQUIRED_IOS_PODS.length} required companion pods, and ${permissions.size} declared permissions.\n`
  );
}

main();

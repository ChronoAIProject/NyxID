#!/usr/bin/env bash
# Bump iOS CFBundleVersion and CURRENT_PROJECT_VERSION by 1.
# Run from mobile/ directory. Syncs Info.plist and project.pbxproj.

set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INFOPLIST="$ROOT/ios/NyxIDMobile/Info.plist"
PBXPROJ="$ROOT/ios/NyxIDMobile.xcodeproj/project.pbxproj"

CURRENT=$(/usr/libexec/PlistBuddy -c "Print CFBundleVersion" "$INFOPLIST")
NEXT=$((CURRENT + 1))

/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $NEXT" "$INFOPLIST"
sed -i '' "s/CURRENT_PROJECT_VERSION = $CURRENT;/CURRENT_PROJECT_VERSION = $NEXT;/g" "$PBXPROJ"

echo "iOS build: $CURRENT → $NEXT"

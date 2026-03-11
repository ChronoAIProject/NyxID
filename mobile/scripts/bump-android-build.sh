#!/usr/bin/env bash
# Bump Android versionCode by 1.
# Run from mobile/ directory. Syncs app.json and android/app/build.gradle.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_JSON="$ROOT/app.json"
BUILD_GRADLE="$ROOT/android/app/build.gradle"

CURRENT=$(node -e "const fs=require('fs'); const data=JSON.parse(fs.readFileSync(process.argv[1], 'utf8')); process.stdout.write(String(data.expo.android.versionCode));" "$APP_JSON")
NEXT=$((CURRENT + 1))

node -e "const fs=require('fs'); const path=process.argv[1]; const next=Number(process.argv[2]); const data=JSON.parse(fs.readFileSync(path, 'utf8')); data.expo.android.versionCode = next; fs.writeFileSync(path, JSON.stringify(data, null, 2) + '\n');" "$APP_JSON" "$NEXT"

sed -i '' "s/versionCode $CURRENT/versionCode $NEXT/" "$BUILD_GRADLE"

echo "Android build: $CURRENT -> $NEXT"

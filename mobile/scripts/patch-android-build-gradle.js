#!/usr/bin/env node
/**
 * EAS post-install patch: force androidx.core to 1.15.0 so prebuild output
 * (compileSdk 35) works without requiring compileSdk 36 / AGP 8.9.1.
 * Run only on Android via eas-build-post-install.
 */
if (process.env.EAS_BUILD_PLATFORM !== 'android') {
  process.exit(0);
}

const fs = require('fs');
const path = require('path');

const buildGradlePath = path.join(__dirname, '..', 'android', 'build.gradle');
if (!fs.existsSync(buildGradlePath)) {
  console.warn('patch-android-build-gradle: android/build.gradle not found, skip');
  process.exit(0);
}

let content = fs.readFileSync(buildGradlePath, 'utf8');

const resolutionBlock = `
    configurations.all {
        resolutionStrategy {
            force 'androidx.core:core:1.15.0'
            force 'androidx.core:core-ktx:1.15.0'
        }
    }
`;

// Inject inside allprojects { } after the repositories { } block
const allprojectsReposEnd = /(allprojects\s*\{\s*repositories\s*\{[\s\S]*?\}\s*)\}/;
if (allprojectsReposEnd.test(content)) {
  content = content.replace(allprojectsReposEnd, `$1${resolutionBlock}\n}`);
  fs.writeFileSync(buildGradlePath, content);
  console.log('patch-android-build-gradle: forced androidx.core to 1.15.0');
} else {
  console.warn('patch-android-build-gradle: could not find allprojects block, skip');
}

const assert = require("node:assert/strict");
const test = require("node:test");
const {
  buildGradleSigningEnvironment,
  redactCommandArgument,
  runFile,
} = require("./android-build-command");

test("redacts Android signing passwords from the displayed Gradle command", () => {
  assert.equal(
    redactCommandArgument(
      "-Pandroid.injected.signing.store.password=store-secret",
    ),
    "-Pandroid.injected.signing.store.password=[redacted]",
  );
  assert.equal(
    redactCommandArgument("-Pandroid.injected.signing.key.password=key-secret"),
    "-Pandroid.injected.signing.key.password=[redacted]",
  );
  assert.equal(
    redactCommandArgument("-Pandroid.injected.signing.key.alias=nyxid"),
    "-Pandroid.injected.signing.key.alias=nyxid",
  );
});

test("builds Gradle signing properties in the child environment", () => {
  const env = buildGradleSigningEnvironment(
    { PATH: "/usr/bin", KEEP: "value" },
    {
      storeFile: "/tmp/release.keystore",
      storePassword: "store-secret",
      keyAlias: "nyxid",
      keyPassword: "key-secret",
    },
  );

  assert.deepEqual(env, {
    PATH: "/usr/bin",
    KEEP: "value",
    "ORG_GRADLE_PROJECT_android.injected.signing.store.file":
      "/tmp/release.keystore",
    "ORG_GRADLE_PROJECT_android.injected.signing.store.password":
      "store-secret",
    "ORG_GRADLE_PROJECT_android.injected.signing.key.alias": "nyxid",
    "ORG_GRADLE_PROJECT_android.injected.signing.key.password": "key-secret",
  });
});

test("passes Gradle arguments as an argv array and keeps signing secrets out of logs", () => {
  const calls = [];
  const logs = [];
  const originalLog = console.log;
  console.log = (message) => logs.push(message);
  try {
    const env = buildGradleSigningEnvironment(
      { PATH: "/usr/bin" },
      {
        storeFile: "/tmp/release.keystore",
        storePassword: "pa$$ with spaces",
        keyAlias: "nyxid",
        keyPassword: "another-secret",
      },
    );
    runFile(
      "/path/to/gradlew",
      [":app:bundleRelease"],
      { cwd: "/tmp", env },
      (...args) => calls.push(args),
    );
  } finally {
    console.log = originalLog;
  }

  assert.deepEqual(calls, [
    [
      "/path/to/gradlew",
      [":app:bundleRelease"],
      {
        cwd: "/tmp",
        env: {
          PATH: "/usr/bin",
          "ORG_GRADLE_PROJECT_android.injected.signing.store.file":
            "/tmp/release.keystore",
          "ORG_GRADLE_PROJECT_android.injected.signing.store.password":
            "pa$$ with spaces",
          "ORG_GRADLE_PROJECT_android.injected.signing.key.alias": "nyxid",
          "ORG_GRADLE_PROJECT_android.injected.signing.key.password":
            "another-secret",
        },
        stdio: "inherit",
      },
    ],
  ]);
  assert.equal(logs.length, 1);
  assert.doesNotMatch(logs[0], /pa\$\$ with spaces|another-secret/);
  assert.match(logs[0], /:app:bundleRelease/);
});

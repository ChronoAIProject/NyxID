const { execFileSync } = require("node:child_process");

const SENSITIVE_GRADLE_ARGUMENTS = new Set([
  "-Pandroid.injected.signing.store.password",
  "-Pandroid.injected.signing.key.password",
]);

const GRADLE_SIGNING_ENV_KEYS = {
  storeFile: "ORG_GRADLE_PROJECT_android.injected.signing.store.file",
  storePassword: "ORG_GRADLE_PROJECT_android.injected.signing.store.password",
  keyAlias: "ORG_GRADLE_PROJECT_android.injected.signing.key.alias",
  keyPassword: "ORG_GRADLE_PROJECT_android.injected.signing.key.password",
};

function redactCommandArgument(argument) {
  const separator = argument.indexOf("=");
  if (separator === -1) return argument;

  const key = argument.slice(0, separator);
  return SENSITIVE_GRADLE_ARGUMENTS.has(key) ? `${key}=[redacted]` : argument;
}

function buildGradleSigningEnvironment(baseEnv, signing) {
  return {
    ...baseEnv,
    [GRADLE_SIGNING_ENV_KEYS.storeFile]: signing.storeFile,
    [GRADLE_SIGNING_ENV_KEYS.storePassword]: signing.storePassword,
    [GRADLE_SIGNING_ENV_KEYS.keyAlias]: signing.keyAlias,
    [GRADLE_SIGNING_ENV_KEYS.keyPassword]: signing.keyPassword,
  };
}

function runFile(command, args, options = {}, executor = execFileSync) {
  console.log(
    `\n$ ${[command, ...args.map(redactCommandArgument)].join(" ")}\n`,
  );
  return executor(command, args, {
    stdio: "inherit",
    ...options,
  });
}

module.exports = {
  buildGradleSigningEnvironment,
  redactCommandArgument,
  runFile,
};

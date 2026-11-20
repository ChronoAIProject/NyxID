import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import { ApiError } from "../lib/api/ApiError";
import { resolveAuthDeviceErrorMessage } from "../lib/api/errorMessages";
import { agentKeyLoginError } from "../features/auth/agentKeyLoginModel";
import simplified from "./locales/zh-CN";
import traditional from "./locales/zh-TW";

function source(path: string) {
  return ts.createSourceFile(
    path,
    readFileSync(new URL(path, import.meta.url), "utf8"),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
}
function visit(node: ts.Node, inspect: (node: ts.Node) => void) {
  inspect(node);
  ts.forEachChild(node, (child) => visit(child, inspect));
}
function literalPhrases(node: ts.Node, phrases: Set<string>) {
  if (ts.isConditionalExpression(node)) {
    // Only the result branches provide copy; the test may compare internal IDs.
    literalPhrases(node.whenTrue, phrases);
    literalPhrases(node.whenFalse, phrases);
    return;
  }
  if (ts.isStringLiteralLike(node)) phrases.add(node.text);
  ts.forEachChild(node, (child) => literalPhrases(child, phrases));
}
function translatedBranches(node: ts.Expression, phrases: Set<string>) {
  if (ts.isStringLiteralLike(node)) phrases.add(node.text);
  else if (ts.isConditionalExpression(node)) {
    translatedBranches(node.whenTrue, phrases);
    translatedBranches(node.whenFalse, phrases);
  }
}

test("every WS2 fixed UI phrase passed to t, including dynamic login, terminal, schema and save copy, has both Chinese translations", () => {
  const phrases = new Set<string>();
  // Scan actual call sites so an added literal or conditional t() phrase cannot
  // silently escape the catalogs. Dynamic message providers are covered below.
  for (const path of [
    "../app/WorkspaceHeader.tsx",
    "../components/settings/Pager.tsx",
    "../features/account/ProfileFieldEditScreen.tsx",
    "../features/agents/AgentsScreen.tsx",
    "../features/assistant/AssistantAttachmentView.tsx",
    "../features/auth/DeviceLoginScreen.tsx",
    "../features/auth/AgentKeyLoginScreen.tsx",
    "../features/auth/LoginRequestTerminalView.tsx",
  ]) {
    visit(source(path), (node) => {
      if (
        ts.isCallExpression(node) &&
        ts.isIdentifier(node.expression) &&
        node.expression.text === "t" &&
        node.arguments[0]
      )
        translatedBranches(node.arguments[0], phrases);
    });
  }
  let loginCopyFound = false;
  visit(source("../features/auth/loginRequestErrors.ts"), (node) => {
    if (
      ts.isFunctionDeclaration(node) &&
      node.name?.text === "loginErrorCopy" &&
      node.body
    ) {
      loginCopyFound = true;
      literalPhrases(node.body, phrases);
    }
  });
  assert.ok(loginCopyFound, "scan the actual login request copy provider");
  const terminalPhrases = new Set<string>();
  visit(source("../features/auth/LoginRequestTerminalView.tsx"), (node) => {
    if (
      ts.isVariableDeclaration(node) &&
      ts.isArrayBindingPattern(node.name) &&
      node.name.elements.some(
        (element) =>
          ts.isBindingElement(element) &&
          element.name.getText() === "explanation",
      ) &&
      node.initializer
    )
      literalPhrases(node.initializer, terminalPhrases);
  });
  for (const phrase of [
    "Return to your browser to continue.",
    "Return to where you started signing in.",
  ])
    assert.ok(terminalPhrases.has(phrase), `scan terminal copy: ${phrase}`);
  assert.equal(
    terminalPhrases.has("browser"),
    false,
    "the client-kind discriminator is not terminal copy",
  );
  for (const phrase of terminalPhrases) phrases.add(phrase);
  for (const path of [
    "../features/assistant/saveAttachmentImage.ts",
    "../lib/api/accountSchema.ts",
    "../lib/api/agentKeyLoginSchema.ts",
  ]) {
    visit(source(path), (node) => {
      if (
        ts.isVariableDeclaration(node) &&
        node.name.getText() === "message" &&
        node.initializer
      )
        translatedBranches(node.initializer, phrases);
      if (
        ts.isPropertyAssignment(node) &&
        ["phrase", "message"].includes(node.name.getText()) &&
        ts.isStringLiteralLike(node.initializer)
      )
        phrases.add(node.initializer.text);
      if (
        ts.isVariableDeclaration(node) &&
        ["IMAGE_NOT_SAVED", "IMAGE_SAVE_FAILED"].includes(
          node.name.getText(),
        ) &&
        node.initializer &&
        ts.isStringLiteralLike(node.initializer)
      )
        phrases.add(node.initializer.text);
      if (
        ts.isCallExpression(node) &&
        ts.isPropertyAccessExpression(node.expression) &&
        ["refine", "regex"].includes(node.expression.name.text) &&
        node.arguments[1] &&
        ts.isStringLiteralLike(node.arguments[1])
      )
        phrases.add(node.arguments[1].text);
    });
  }
  for (let errorCode = 11200; errorCode <= 11207; errorCode++)
    phrases.add(
      resolveAuthDeviceErrorMessage(
        new ApiError({
          errorCode,
          errorKey: "device_error",
          statusCode: 400,
          message: "untrusted server message",
        }),
      ),
    );
  phrases.add(resolveAuthDeviceErrorMessage(new Error("network")));
  for (const errorCode of [
    2003, 11900, 11901, 11902, 11903, 11904, 11905, 11906, 11907, 11908, 11909,
  ])
    phrases.add(agentKeyLoginError({ errorCode }));
  for (const phrase of [
    "Could not reach NyxID. Try again.",
    "Check the login details.",
    "Check this value.",
    "Display name must be 200 bytes or less",
    "Credential expiry cannot exceed key expiry.",
    "Enter an expiry as YYYY-MM-DD or an RFC 3339 timestamp with a timezone.",
  ])
    phrases.add(phrase);
  assert.ok(
    phrases.size > 50,
    "include both literal UI and dynamic runtime copy",
  );
  for (const [language, catalog] of [
    ["zh-CN", simplified],
    ["zh-TW", traditional],
  ] as const) {
    for (const phrase of phrases) {
      assert.ok(
        Object.hasOwn(catalog, phrase),
        `${language}: missing ${phrase}`,
      );
      const translation = (catalog as Record<string, string>)[phrase]!;
      assert.ok(translation.trim(), `${language}: empty ${phrase}`);
      assert.notEqual(
        translation,
        phrase,
        `${language}: English fallback for ${phrase}`,
      );
      assert.match(
        translation.replace(/\{\{\w+\}\}/g, ""),
        /[^\x00-\x7f]/,
        `${language}: non-English translation for ${phrase}`,
      );
    }
    assert.equal(Object.hasOwn(catalog, "Shared to {{app}}."), false);
  }
});

#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const locales = path.join(root, "src/i18n/locales");
const protocolExamples = new Set(JSON.parse(fs.readFileSync(path.join(root, "src/i18n/protocol-examples.json"), "utf8")));
const glossary = JSON.parse(fs.readFileSync(path.join(root, "src/i18n/glossary.json"), "utf8"));
const simplifiedOnly = new Set(JSON.parse(fs.readFileSync(path.join(root, "src/i18n/simplified-only.json"), "utf8")));
const domainGlossary = JSON.parse(fs.readFileSync(path.join(root, "src/i18n/domain-glossary.json"), "utf8"));
const issues = [];
const catalogs = new Map();
const fullEnglish = JSON.parse(fs.readFileSync(path.join(locales, "en.json"), "utf8"));
const englishOverrides = JSON.parse(fs.readFileSync(path.join(locales, "en-overrides.json"), "utf8"));
const expectedOverrides = Object.fromEntries(Object.entries(fullEnglish).filter(([key, value]) => key !== value));
if (JSON.stringify(englishOverrides) !== JSON.stringify(expectedOverrides)) {
  issues.push("English runtime overrides are stale; run npm run sync:i18n");
}
const placeholders = (text) =>
  [...text.matchAll(/{{\s*([^}]+?)\s*}}/g)].map((match) => match[1]).sort().join("\0");
const htmlEntity = /&(?:[a-z]+|#\d+|#x[0-9a-f]+);/i;

for (const prefix of ["", "wizard-", "common-", "errors-"]) {
  const english = JSON.parse(fs.readFileSync(path.join(locales, `${prefix}en.json`), "utf8"));
  catalogs.set(prefix, english);
  for (const key of protocolExamples) {
    if (key in english) issues.push(`${prefix}en: protocol example must stay outside catalogs: ${key}`);
  }
  for (const [key, value] of Object.entries(english)) {
    if (htmlEntity.test(key) || htmlEntity.test(value)) issues.push(`${prefix}en: use literal characters instead of HTML entities: ${key}`);
  }
  for (const language of ["zh-CN", "zh-TW"]) {
    const translated = JSON.parse(fs.readFileSync(path.join(locales, `${prefix}${language}.json`), "utf8"));
    for (const key of new Set([...Object.keys(english), ...Object.keys(translated)])) {
      if (!(key in english) || !translated[key]?.trim()) {
        issues.push(`${prefix}${language}: missing or empty translation: ${key}`);
      } else if (placeholders(english[key]) !== placeholders(translated[key])) {
        issues.push(`${prefix}${language}: interpolation differs: ${key}`);
      }
      if (htmlEntity.test(translated[key] ?? "")) issues.push(`${prefix}${language}: use literal characters instead of HTML entities: ${key}`);
      for (const term of glossary) {
        if (new RegExp(`\\b${term}\\b`).test(english[key] ?? "") && !translated[key]?.includes(term)) {
          issues.push(`${prefix}${language}: preserve ${term}: ${key}`);
        }
      }
      if (language === "zh-TW" && [...(translated[key] ?? "")].some((character) => simplifiedOnly.has(character))) {
        issues.push(`${prefix}${language}: Simplified-only character in Traditional translation: ${key}`);
      }
      for (const rule of domainGlossary) {
        if (new RegExp(rule.source, "i").test(key) &&
          !(rule.except && new RegExp(rule.except, "i").test(key)) &&
          new RegExp(rule.forbidden, "u").test(translated[key] ?? "")) {
          issues.push(`${prefix}${language}: incorrect ${rule.term} terminology: ${key}`);
        }
      }
    }
  }
}

const displayProps = new Set([
  "title", "description", "label", "hint", "placeholder", "aria-label", "alt",
  "emptyMessage", "emptyTitle", "emptyDescription", "confirmLabel", "cancelLabel",
  "loadingLabel", "message", "listLabel", "errorMessage", "noResultsMessage", "ariaLabel",
  "searchPlaceholder", "heading", "subtitle", "triggerLabel", "desc",
  "compactLabel", "serviceName", "personalLabel", "itemLabel", "emptyLabel",
  "activeLabel", "inactiveLabel", "submitLabel", "copyLabel", "allLabel", "firstHeading", "error",
  "footer", "caption", "saveLabel", "discardLabel", "tooltip", "imageWarning",
  "ackButtonLabel", "ownerLabel", "backLabel", "defaultLabel", "contentTitle", "actionLabel", "groupLabel", "currentLabel", "customPlaceholder", "activePlaceholder", "sendLabel", "connectionLabel", "showLabel", "subtitleTitle",
]);
const isDisplayProp = (attribute) => displayProps.has(attribute.name.getText()) ||
  attribute.name.getText() === "value" &&
  ["StatusCell", "StatCard"].includes(attribute.parent.parent.tagName?.getText());
// These literals are intentionally shown verbatim as protocol examples or names.
const verbatim = new Set([
  "NyxID", "ABCD-1234", "user:pass", "sk-...", "role-id-1,role-id-2",
  "openid profile email",
]);
const decodeEntities = (text) => text.replace(
  /&(amp|quot|apos|lt|gt|ldquo|rdquo|middot|rarr|#39);/g,
  (_, name) => ({ amp: "&", quot: '"', apos: "'", lt: "<", gt: ">", ldquo: "“", rdquo: "”", middot: "·", rarr: "→", "#39": "'" })[name],
);
const isCopy = (text) => /[A-Za-z]{2}/.test(text) &&
  !verbatim.has(text) && !protocolExamples.has(text) && !/^(?:https?:|\/|~\/|\.(?!\s))/.test(text);
function sourceFiles(dir) {
  return fs.readdirSync(path.join(root, dir), { withFileTypes: true }).flatMap((entry) => {
    const file = `${dir}/${entry.name}`;
    return entry.isDirectory() ? sourceFiles(file) : entry.isFile() ? [file] : [];
  });
}
const files = sourceFiles("src").filter((file) => /\.tsx?$/.test(file) &&
    !/\.test\.|fixtures?|mock|\/dev\/|\/locales\/|features\/(?:landing|nyxbot-onboarding)\//.test(file));
let checkedKeys = 0;
const errorHelperFiles = new Set([
  "src/schemas/auth-device.ts", "src/schemas/agent-key-login.ts",
  "src/lib/assistant/errors.ts", "src/lib/assistant/request-error.ts",
  "src/lib/voice-start-error.ts",
]);
const hasKey = (key) => [...catalogs.values()].some((catalog) => key in catalog);

for (const file of files) {
  const source = fs.readFileSync(path.join(root, file), "utf8");
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true);
  const report = (node, message) => {
    const line = ast.getLineAndCharacterOfPosition(node.getStart()).line + 1;
    issues.push(`${file}:${line}: ${message}`);
  };
  const withinCode = (node) => {
    for (let parent = node.parent; parent; parent = parent.parent) {
      if (ts.isJsxElement(parent) && ["code", "pre", "svg", "script", "style"].includes(parent.openingElement.tagName.getText(ast))) return true;
    }
    return false;
  };
  const visit = (node) => {
    if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression) &&
      /^toLocale(?:Date|Time)?String$/.test(node.expression.name.text) &&
      (!node.arguments.length || node.arguments[0].getText(ast) === "undefined" ||
        ts.isArrayLiteralExpression(node.arguments[0]) && !node.arguments[0].elements.length)) {
      report(node, "format displayed dates and numbers with the selected interface locale");
    }
    if (ts.isNewExpression(node) && /^Intl\.(?:Number|DateTime|RelativeTime)Format$/.test(node.expression.getText(ast)) &&
      (!node.arguments?.length || node.arguments[0].getText(ast) === "undefined" ||
        ts.isArrayLiteralExpression(node.arguments[0]) && !node.arguments[0].elements.length)) {
      report(node, "format displayed dates and numbers with the selected interface locale");
    }
    if (ts.isStringLiteral(node) && errorHelperFiles.has(file) &&
      /[A-Za-z]{3} [A-Za-z]{2}/.test(node.text) && !hasKey(node.text)) {
      report(node, `missing error catalog key: ${node.text}`);
    }
    if (ts.isCallExpression(node) && node.expression.getText(ast) === "toast.error") {
      const argument = node.arguments[0];
      if (argument && !(ts.isCallExpression(argument) &&
        /^(t|translateUi|translateErrorMessage)$/.test(argument.expression.getText(ast)))) {
        report(node, "translate error toasts at presentation");
      }
    }
    if (ts.isCallExpression(node) && /^(t|translate|translateUi|translateWizard|translateErrorMessage)$/.test(node.expression.getText(ast))) {
      const argument = node.arguments[0];
      const checkKey = (key) => {
        if (ts.isStringLiteral(key) && key.text) {
          checkedKeys++;
          if (protocolExamples.has(key.text)) report(node, `protocol example sent to translation: ${key.text}`);
          if (![...catalogs.values()].some((catalog) => key.text in catalog)) report(node, `missing catalog key: ${key.text}`);
        } else if (ts.isConditionalExpression(key)) {
          checkKey(key.whenTrue);
          checkKey(key.whenFalse);
        } else if (ts.isTemplateExpression(key)) {
          report(node, "use a literal translation key with named interpolation values");
        }
      };
      if (argument) checkKey(argument);
      for (let parent = node.parent; parent; parent = parent.parent) {
        if (ts.isBinaryExpression(parent) && ["===", "!==", "==", "!="].includes(parent.operatorToken.getText(ast))) {
          report(node, "compare stable values before translating their display labels");
          break;
        }
        if (ts.isJsxAttribute(parent)) {
          const name = parent.name.getText(ast);
          const tag = parent.parent.parent.tagName?.getText(ast) ?? "";
          const inputValue = name === "value" && (/^[a-z]/.test(tag) || ["Input", "Textarea", "Select", "SelectItem", "Tabs", "TabsTrigger", "RadioGroup", "RadioGroupItem"].includes(tag));
          if (inputValue || ["key", "id", "name", "href", "to", "htmlFor", "className"].includes(name)) report(node, "translation used in an operational JSX property");
          break;
        }
        if (ts.isStatement(parent) || ts.isJsxElement(parent) || ts.isFunctionLike(parent)) break;
      }
    }
    let copy;
    if (ts.isJsxText(node)) copy = node.text.trim().replace(/\s+/g, " ");
    else if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
      let parent = node.parent;
      if (ts.isJsxAttribute(parent) && isDisplayProp(parent)) copy = node.text;
      else {
        while (ts.isParenthesizedExpression(parent) ||
          ts.isConditionalExpression(parent) && parent.condition !== node ||
          ts.isBinaryExpression(parent) && ["||", "??"].includes(parent.operatorToken.getText(ast))) parent = parent.parent;
        if (ts.isJsxExpression(parent) && (!ts.isJsxAttribute(parent.parent) || isDisplayProp(parent.parent))) copy = node.text;
      }
    }
    if (copy && isCopy(decodeEntities(copy)) && !withinCode(node)) report(node, `untranslated UI literal: ${copy}`);
    ts.forEachChild(node, visit);
  };
  visit(ast);
}

if (issues.length) {
  console.error(issues.join("\n"));
  process.exitCode = 1;
} else {
  console.log(`i18n: ${files.length} source files, ${checkedKeys} static calls; catalogs and visible literals passed.`);
}

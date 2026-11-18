#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { identicalDuplicateWarnings } from "../../shared/i18n/catalog-core.mjs";
import {
  FIXTURE_SOURCE, LANGUAGES, LEGACY_CATALOG_IMPORTS, SPECIFIER_ALIASES, allCatalogFiles, analyzePortions,
  canonicalText, catalogImportIssue, englishOverrides as makeOverrides, isTestSource, legacyImportReplacement,
  loadPortions, mayImportLegacyCatalog, moduleSpecifierOf, readJson,
} from "./lib/i18n-catalogs.mjs";
import { portionForPath } from "./lib/i18n-portions.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const locales = path.join(root, "src/i18n/locales");
const protocolExamples = new Set(JSON.parse(fs.readFileSync(path.join(root, "src/i18n/protocol-examples.json"), "utf8")));
const glossary = JSON.parse(fs.readFileSync(path.join(root, "src/i18n/glossary.json"), "utf8"));
const simplifiedOnly = new Set(JSON.parse(fs.readFileSync(path.join(root, "src/i18n/simplified-only.json"), "utf8")));
const domainGlossary = JSON.parse(fs.readFileSync(path.join(root, "src/i18n/domain-glossary.json"), "utf8"));
const issues = [];
const catalogs = new Map();
const portions = loadPortions(locales);
const analysis = analyzePortions(portions);
issues.push(...analysis.issues);
for (const warning of identicalDuplicateWarnings(analysis, "run npm run sync:i18n")) console.warn(warning);
for (const file of allCatalogFiles(locales)) {
  try {
    if (fs.readFileSync(file, "utf8") !== canonicalText(readJson(file))) issues.push(`${path.relative(root, file)}: non-canonical catalog; run npm run sync:i18n`);
  } catch (error) {
    issues.push(`${path.relative(root, file)}: invalid catalog (${error.message})`);
  }
}
const adoptHint = "run npm run adopt:i18n (see docs/MULTILINGUAL.md for explicit --branch/--base/--onto)";
if (fs.existsSync(path.join(locales, "en-overrides.json"))) issues.push(`legacy file en-overrides.json is present; ${adoptHint}`);
for (const language of LANGUAGES) {
  if (fs.existsSync(path.join(locales, `${language}.json`))) issues.push(`legacy monolith ${language}.json is present; ${adoptHint}`);
  for (const portion of Object.keys(portions)) {
    if (!fs.existsSync(path.join(locales, "app", portion, `${language}.json`))) issues.push(`app/${portion}: missing ${language}.json`);
    else if (!Object.keys(portions[portion][language]).length) issues.push(`app/${portion}/${language}.json: empty area; run npm run sync:i18n`);
  }
}
const fullEnglish = analysis.merged.en;
const overridesFile = path.join(locales, "app/en-overrides.json");
if (!fs.existsSync(overridesFile) || canonicalText(readJson(overridesFile)) !== canonicalText(makeOverrides(fullEnglish))) {
  issues.push("English runtime overrides are stale; run npm run sync:i18n");
}
const placeholders = (text) =>
  [...text.matchAll(/{{\s*([^}]+?)\s*}}/g)].map((match) => match[1]).sort().join("\0");
const htmlEntity = /&(?:[a-z]+|#\d+|#x[0-9a-f]+);/i;

for (const prefix of ["", "wizard-", "common-", "errors-"]) {
  const english = prefix ? readJson(path.join(locales, `${prefix}en.json`)) : fullEnglish;
  catalogs.set(prefix, english);
  for (const key of protocolExamples) {
    if (key in english) issues.push(`${prefix}en: protocol example must stay outside catalogs: ${key}`);
  }
  for (const [key, value] of Object.entries(english)) {
    if (htmlEntity.test(key) || htmlEntity.test(value)) issues.push(`${prefix}en: use literal characters instead of HTML entities: ${key}`);
  }
  for (const language of ["zh-CN", "zh-TW"]) {
    const translated = prefix ? readJson(path.join(locales, `${prefix}${language}.json`)) : analysis.merged[language];
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
// The import boundary covers all non-test source, including fixtures and locale barrels;
// the copy audit covers a narrower list. Each file is parsed once for both. Legacy
// catalog imports are reported in every source, tests included; test files are only
// parsed when their text names a legacy catalog file.
const allSourceFiles = sourceFiles("src").filter((file) => /\.tsx?$/.test(file));
const guardedFiles = allSourceFiles.filter((file) => !isTestSource(file));
// The bilingual public website owns route-localized copy, like the existing landing page.
const isAuditFile = (file) => !FIXTURE_SOURCE.test(file) &&
  !/\.test\.|\/dev\/|\/locales\/|features\/(?:landing|nyxbot-onboarding|website-a)\//.test(file);
const files = guardedFiles.filter(isAuditFile);
let checkedKeys = 0;
const errorHelperFiles = new Set([
  "src/lib/api-error.ts",
  "src/schemas/auth-device.ts", "src/schemas/agent-key-login.ts",
  "src/lib/assistant/errors.ts", "src/lib/assistant/request-error.ts",
  "src/lib/voice-start-error.ts",
]);
const hasKey = (key) => [...catalogs.values()].some((catalog) => key in catalog);

for (const file of allSourceFiles) {
  const source = fs.readFileSync(path.join(root, file), "utf8");
  const test = isTestSource(file);
  if (test && !mayImportLegacyCatalog(source, LEGACY_CATALOG_IMPORTS)) continue;
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true);
  const audit = !test && isAuditFile(file);
  const report = (node, message) => {
    const line = ast.getLineAndCharacterOfPosition(node.getStart()).line + 1;
    issues.push(`${file}:${line}: ${message}`);
  };
  const guardImport = (node) => {
    const specifier = moduleSpecifierOf(node, ast);
    if (!specifier) return;
    const replacement = legacyImportReplacement(file, specifier.text, LEGACY_CATALOG_IMPORTS, SPECIFIER_ALIASES);
    if (replacement) report(node, `legacy catalog import ${specifier.text}; use ${replacement} (npm run adopt:i18n rewrites it; outside a merge or rebase run npm run adopt:i18n -- --imports-only)`);
    const issue = test ? null : catalogImportIssue(file, specifier.text);
    if (issue) report(node, issue);
  };
  const withinCode = (node) => {
    for (let parent = node.parent; parent; parent = parent.parent) {
      if (ts.isJsxElement(parent) && ["code", "pre", "svg", "script", "style"].includes(parent.openingElement.tagName.getText(ast))) return true;
    }
    return false;
  };
  const auditNode = (node) => {
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
          if (![...catalogs.values()].some((catalog) => key.text in catalog)) report(node, `missing catalog key: ${key.text}; suggested app portion: ${portionForPath(file)}`);
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
  };
  const visit = (node) => {
    guardImport(node);
    if (audit) auditNode(node);
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

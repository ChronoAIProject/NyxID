import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import ts from 'typescript';
import postcss from 'postcss';

// These routes do not mount useApplyTheme. Legal/docs/auth-layout/wizard ARE themed.
export const unthemed = new Set([
  'pages/cli-auth.tsx', 'pages/login-agent-key.tsx', 'pages/login-device.tsx',
  'pages/oauth-complete.tsx', 'pages/oauth-launching.tsx',
  'pages/platform-personal-link.tsx', 'pages/ssh-terminal.tsx',
  ...['agent-key-create-form', 'agent-key-permissions', 'device-approval',
    'login-access-rows', 'login-actions', 'login-code-status',
    'login-connection-choices', 'login-grant-review', 'login-identity-card',
    'login-key-draft', 'login-permission-picker', 'login-request-preview',
  ].map(name => `components/auth/${name}.tsx`),
]);
const excluded = file => /\.test\.|\.spec\.|\.d\.ts$/.test(file) || unthemed.has(file) || /^features\/(landing|blog)\//.test(file);

export function typographyViolations(file, source) {
  const errors = [];
  if (!excluded(file) && source.includes('--assistant-density')) errors.push('shared spacing must use --display-density');
  const fail = (offset, message) => errors.push(`line ${source.slice(0, offset).split('\n').length}: ${message}`);
  const classes = (text, offset) => {
    if (!excluded(file)) {
      const themed = text.split(/\s+/).filter(token => !token.startsWith('unthemed:'))
        .map(token => token.replace(/(^|:)!/g, '$1').replace(/!$/, '')).join(' ');
      if (/(?:^|\s)(?:\S*:)?leading-/.test(themed)) fail(offset, 'line-height belongs to a shared typography role');
      if (/(?:^|\s)(?:\S*:)?type-[\w-]+\b/.test(themed) && /(?:^|\s)(?:(?!\[)[\w-]+:)*text-(?:\d+\b|\[[.\d]+(?:px|rem|em)\]|\[(?:length:|[^\]]*--text-)[^\]]*\])/.test(themed)) {
        fail(offset, 'complete typography roles cannot be combined with partial size assignments');
      }
      if (/(?:^|\s)(?:\S*:)?(?:text-\d+|type-[\w-]+)\b/.test(themed) && /(?:^|\s)(?:\S*:)?font-(?:thin|extralight|light|normal|medium|semibold|bold|extrabold|black|\[[^\]]+\])(?:\s|$)/.test(themed)) {
        fail(offset, 'size, weight and line-height must come from one shared typography role');
      }
    }
    if (/\bfont-display\b/.test(text) && /\btext-28\b/.test(text) && !/\bfont-(semibold|bold)\b/.test(text)) {
      fail(offset, 'prominent metrics use type-metric (page titles use type-page-title)');
    }
    for (const match of text.matchAll(/(?<![\w-])(?:[^\s"'`]*:)?text-(?:9|10|11|xs|sm|base|lg|xl|[2-9]xl)(?![\w-])|text-\[(?:[.\d]+(?:px|rem|em))\]/g)) {
      if (match[0].includes('unthemed:')) continue;
      fail(offset + match.index, `${match[0]} must use a numeric semantic role (minimum text-12)`);
    }
  };
  if (file.endsWith('.css')) {
    const css = postcss.parse(source);
    if (!excluded(file) && file !== 'typography.css') css.walkRules(rule => {
      const applies = rule.nodes.filter(node => node.type === 'atrule' && node.name === 'apply');
      if (applies.length > 1) classes(applies.map(node => node.params).join(' '), rule.source.start.offset);
      if (rule.nodes.some(node => node.type === 'atrule' && node.name === 'apply' && /\btype-[\w-]+/.test(node.params))) {
        for (const node of rule.nodes) if (node.type === 'decl' && /^(font|font-size)$/.test(node.prop)) {
          fail(node.source.start.offset, 'complete typography roles cannot be overridden by font declarations');
        }
      }
    });
    css.walkRules(rule => {
      if (!/\.nb-(primary|secondary|provider)\b|\.nb-device-login\s*>\s*button/.test(rule.selector)) return;
      const declarations = Object.fromEntries(rule.nodes.filter(node => node.type === 'decl').map(node => [node.prop, node.value]));
      if (rule.nodes.some(node => node.type === 'atrule' && node.name === 'apply' && node.params === 'type-button')) return;
      if ('font-size' in declarations && (declarations['font-size'] !== 'var(--text-14)' || declarations['font-weight'] !== '500')) {
        errors.push(`${rule.selector}: custom onboarding actions must use the button role, text-14 / 500`);
      }
    });
    css.walkDecls(decl => {
      if (/^--text-\d+$/.test(decl.prop)) {
        const expected = `${Number(decl.prop.slice(7)) / 16}rem`;
        if (decl.value !== expected) errors.push(`${decl.prop} must equal ${expected}`);
        if (file !== 'app.css' || decl.parent.type !== 'atrule' || decl.parent.name !== 'theme') errors.push(`${decl.prop} may only be defined in app.css @theme`);
      }
      if (!excluded(file) && /^(font|font-size)$/.test(decl.prop) && !decl.value.includes('var(--text-') && !/^(inherit|initial|unset)$/.test(decl.value)) errors.push(`${decl.prop}: ${decl.value} must use a numeric token`);
      if (!excluded(file) && /var\(--text-(9|10|11)\)/.test(decl.value)) errors.push(`${decl.value} is below the readable role minimum`);
      if (!excluded(file) && file !== 'typography.css' && decl.prop === 'font' && !/^(inherit|initial|unset)$/.test(decl.value)) errors.push('font shorthands bypass role line-height; use a shared role and font-family');
      if (!excluded(file) && file !== 'typography.css' && decl.prop === 'font-weight' && !(decl.parent.type === 'atrule' && decl.parent.name === 'font-face')) errors.push('font weight belongs to a shared typography role');
      if (/^--line-/.test(decl.prop) && file !== 'typography.css') errors.push('role line-height definitions belong only in typography.css');
      if (!excluded(file) && decl.prop === 'line-height' && !/^(?:inherit|normal|var\(--line-(?:body|public-body|title|page-title|metric|prose)\))$/.test(decl.value)) errors.push(`${decl.value}: line-height must consume a shared role`);
    });
    css.walkAtRules('apply', rule => classes(rule.params, rule.source.start.offset));
  } else if (!excluded(file)) {
    // Most modules cannot contain a violation. Keep the repository scan cheap
    // enough for the default test timeout, including on a busy CI worker.
    const needsAst = /<label\b|<legend\b|fontSize|fontWeight|font-display|\bfont\s*:|\blineHeight\s*:|leading-|type-|TEXT_STYLES|(?:Label|ui\/label)|text-(?:9|10|11|xs|sm|base|lg|xl|[2-9]xl|\[)/.test(source)
      || (/font-(?:thin|extralight|light|normal|medium|semibold|bold|extrabold|black|\[)/.test(source) && /text-\d|type-|TEXT_STYLES\./.test(source));
    if (!needsAst) return errors;
    const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, file.endsWith('tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    const constants = new Map();
    const labelNames = new Set(['Label', 'FormLabel', 'GroupedLabel']);
    function findConstants(node) {
      if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer) constants.set(node.name.text, node.initializer);
      if (ts.isImportDeclaration(node) && /(?:^|\/)(?:label|form|grouped-fields)$/.test(node.moduleSpecifier.text)) {
        const bindings = node.importClause?.namedBindings;
        if (bindings && ts.isNamedImports(bindings)) for (const item of bindings.elements) {
          if (labelNames.has((item.propertyName ?? item.name).text)) labelNames.add(item.name.text);
        }
        if (bindings && ts.isNamespaceImport(bindings)) for (const name of ['Label', 'FormLabel', 'GroupedLabel']) labelNames.add(`${bindings.name.text}.${name}`);
      }
      ts.forEachChild(node, findConstants);
    }
    findConstants(ast);
    function classFragments(node, seen = new Set()) {
      if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node) || ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)) return [node.text];
      if (ts.isIdentifier(node) && constants.has(node.text) && !seen.has(node.text)) {
        return classFragments(constants.get(node.text), new Set([...seen, node.text]));
      }
      if (ts.isPropertyAccessExpression(node) && node.expression.getText(ast) === 'TEXT_STYLES') return ['type-shared-role'];
      const fragments = [];
      ts.forEachChild(node, child => { fragments.push(...classFragments(child, seen)); });
      return fragments;
    }
    const combine = lists => lists.reduce((left, right) => left.flatMap(a => right.map(b => `${a} ${b}`)), ['']);
    function classVariants(node, seen = new Set()) {
      if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return [node.text];
      if (ts.isIdentifier(node) && constants.has(node.text) && !seen.has(node.text)) return classVariants(constants.get(node.text), new Set([...seen, node.text]));
      if (ts.isConditionalExpression(node)) return [...classVariants(node.whenTrue, seen), ...classVariants(node.whenFalse, seen)];
      if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken) return ['', ...classVariants(node.right, seen)];
      if (ts.isPropertyAccessExpression(node) && node.expression.getText(ast) === 'TEXT_STYLES') return ['type-shared-role'];
      if (ts.isCallExpression(node)) return combine(node.arguments.map(arg => classVariants(arg, seen)));
      if (ts.isTemplateExpression(node)) return combine([[node.head.text], ...node.templateSpans.flatMap(span => [classVariants(span.expression, seen), [span.literal.text]])]);
      if (ts.isElementAccessExpression(node) && ts.isIdentifier(node.expression) && constants.has(node.expression.text)) {
        const object = constants.get(node.expression.text);
        if (ts.isObjectLiteralExpression(object)) return object.properties.flatMap(property => ts.isPropertyAssignment(property) ? classVariants(property.initializer, seen) : ['']);
      }
      return classFragments(node, seen);
    }
    const isOpening = node => ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node);
    const isLabel = node => isOpening(node) && labelNames.has(node.tagName.getText(ast));
    const typography = /(?:type-[\w-]+|text-(?:\d+|\[(?:length:|[^\]]*--text-))|font-(?:thin|extralight|light|normal|medium|semibold|bold|extrabold|black|\[)|leading-)/;
    function styleOverrides(node, seen = new Set()) {
      if (ts.isIdentifier(node) && constants.has(node.text) && !seen.has(node.text)) return styleOverrides(constants.get(node.text), new Set([...seen, node.text]));
      if ((ts.isPropertyAssignment(node) || ts.isShorthandPropertyAssignment(node)) && /^(?:font|fontSize|fontWeight|lineHeight)$/.test(node.name.getText(ast).replace(/["']/g, ''))) return true;
      let found = false;
      ts.forEachChild(node, child => { if (styleOverrides(child, seen)) found = true; });
      return found;
    }
    // Controls and explicitly separated help text own their roles; caption
    // descendants (including asChild) must retain the shared label role.
    function insideLabelCaption(node) {
      for (let parent = node.parent; parent; parent = parent.parent) {
        if (!ts.isJsxElement(parent)) continue;
        const opening = parent.openingElement;
        if (isLabel(opening)) return true;
        if (/^(?:GroupedInput|GroupedTextarea|Input|Textarea|NativeSelect|Select\w*|Button|Badge|FormControl|FormDescription|GroupedDescription|FieldHelp|input|textarea|select|button)$/.test(opening.tagName.getText(ast))) return false;
      }
      return false;
    }
    function visit(node) {
      if ((ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) && node.tagName.getText(ast) === 'label') fail(node.getStart(ast), 'form labels must use the shared Label/FormLabel/GroupedLabel implementation');
      if (ts.isJsxOpeningElement(node) && node.tagName.getText(ast) === 'legend' && !(ts.isJsxElement(node.parent.parent) && node.parent.parent.openingElement.tagName.getText(ast) === 'Label')) fail(node.getStart(ast), 'fieldset captions must use Label asChild to retain native legend semantics');
      if (isOpening(node)) {
        const attribute = node.attributes.properties.find(property => ts.isJsxAttribute(property) && property.name.getText(ast) === 'className');
        const style = node.attributes.properties.find(property => ts.isJsxAttribute(property) && property.name.getText(ast) === 'style');
        const spreadOverride = node.attributes.properties.some(property => ts.isJsxSpreadAttribute(property) && styleOverrides(property.expression));
        const fragments = attribute ? classFragments(attribute).join(' ').replace(/unthemed:[\w-]+/g, '') : '';
        const caption = isLabel(node) || (!/^(?:GroupedInput|GroupedTextarea|Input|Textarea|NativeSelect|Select\w*|Button|Badge|FormControl|FormDescription|GroupedDescription|FieldHelp|input|textarea|select|button)$/.test(node.tagName.getText(ast)) && insideLabelCaption(node));
        if (caption && ((attribute && typography.test(fragments)) || (style && styleOverrides(style)) || spreadOverride)) fail(node.getStart(ast), 'field labels and their captions own their typography; consumers may only customize layout and color');
        else if (/\btype-[\w-]+/.test(fragments) && ((style && styleOverrides(style)) || spreadOverride)) fail(node.getStart(ast), 'complete typography roles cannot be overridden by inline typography');
      }
      if (ts.isJsxAttribute(node) && node.name.getText(ast) === 'className' && node.initializer && ts.isJsxExpression(node.initializer)) {
        for (const variant of classVariants(node.initializer.expression ?? node.initializer)) classes(variant, node.getStart(ast));
      }
      if (ts.isTemplateExpression(node) && node.getText(ast).includes('TEXT_STYLES.')) {
        classes(node.head.text + node.templateSpans.map(span => `${span.expression.getText(ast).startsWith('TEXT_STYLES.') ? ' type-shared-role ' : ' '}${span.literal.text}`).join(''), node.getStart(ast));
      }
      if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node) || ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)) classes(node.text, node.getStart(ast));
      if ((ts.isPropertyAssignment(node) && /^(fontSize|font)$/.test(node.name.getText(ast))) || (ts.isJsxAttribute(node) && node.name.getText(ast) === 'fontSize')) {
        const value = (node.initializer?.getText(ast) ?? '');
        if (!/var\(--text-|computedTextSize\(|^tokenSize$|inherit/.test(value)) fail(node.getStart(ast), `${node.name.getText(ast)} must use a numeric token, computedTextSize for canvas consumers`);
      }
      if (ts.isPropertyAssignment(node) && node.name.getText(ast) === 'lineHeight' && !/computedTextLineHeight\(/.test(node.initializer.getText(ast))) fail(node.getStart(ast), 'library line-height must consume computedTextLineHeight');
      ts.forEachChild(node, visit);
    }
    visit(ast);
  }
  return errors;
}

export function checkTypography(root) {
  const errors = [];
  let definitions = [];
  for (const file of readdirSync(root, { recursive: true }).filter(file => /\.(tsx?|css)$/.test(file))) {
    const source = readFileSync(resolve(root, file), 'utf8');
    errors.push(...typographyViolations(file, source).map(error => `${file}: ${error}`));
    if (file === 'app.css') definitions = [...source.matchAll(/--text-(\d+)\s*:/g)].map(match => match[1]);
  }
  if (new Set(definitions).size !== definitions.length) errors.push('app.css: text tokens must have exactly one definition, inside @theme');
  return errors;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const errors = checkTypography(fileURLToPath(new URL('../src/', import.meta.url)));
  if (errors.length) {
    process.stderr.write(errors.join('\n') + '\n');
    process.exitCode = 1;
  } else process.stdout.write('Typography guard passed: canonical definitions, numeric roles, scaling CSS and library consumers.\n');
}

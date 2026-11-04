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
  const fail = (offset, message) => errors.push(`line ${source.slice(0, offset).split('\n').length}: ${message}`);
  const classes = (text, offset) => {
    if (/\bfont-display\b/.test(text) && /\btext-28\b/.test(text) && !/\bfont-(semibold|bold)\b/.test(text)) {
      fail(offset, 'prominent metrics use text-28 font-semibold (page titles use font-bold)');
    }
    for (const match of text.matchAll(/(?<![\w-])(?:[^\s"'`]*:)?text-(?:9|10|11|xs|sm|base|lg|xl|[2-9]xl)(?![\w-])|text-\[(?:[.\d]+(?:px|rem|em))\]/g)) {
      if (match[0].includes('unthemed:')) continue;
      fail(offset + match.index, `${match[0]} must use a numeric semantic role (minimum text-12)`);
    }
  };
  if (file.endsWith('.css')) {
    const css = postcss.parse(source);
    css.walkRules(rule => {
      if (!/\.nb-(primary|secondary|provider)\b|\.nb-device-login\s*>\s*button/.test(rule.selector)) return;
      const declarations = Object.fromEntries(rule.nodes.filter(node => node.type === 'decl').map(node => [node.prop, node.value]));
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
    });
    css.walkAtRules('apply', rule => classes(rule.params, rule.source.start.offset));
  } else if (!excluded(file) && /fontSize|font-display|(?<![\w-])font\s*:|text-(?:9|10|11|xs|sm|base|lg|xl|[2-9]xl|\[)/.test(source)) {
    const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, file.endsWith('tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    function visit(node) {
      if (ts.isJsxAttribute(node) && node.name.getText(ast) === 'className' && node.initializer && ts.isJsxExpression(node.initializer)) {
        const fragments = [];
        function collect(child) {
          if (ts.isStringLiteral(child) || ts.isNoSubstitutionTemplateLiteral(child)) fragments.push(child.text);
          ts.forEachChild(child, collect);
        }
        collect(node.initializer);
        classes(fragments.join(' '), node.getStart(ast));
      }
      if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node) || ts.isTemplateHead(node) || ts.isTemplateMiddle(node) || ts.isTemplateTail(node)) classes(node.text, node.getStart(ast));
      if ((ts.isPropertyAssignment(node) && /^(fontSize|font)$/.test(node.name.getText(ast))) || (ts.isJsxAttribute(node) && node.name.getText(ast) === 'fontSize')) {
        const value = (node.initializer?.getText(ast) ?? '');
        if (!/var\(--text-|computedTextSize\(|^tokenSize$|inherit/.test(value)) fail(node.getStart(ast), `${node.name.getText(ast)} must use a numeric token, computedTextSize for canvas consumers`);
      }
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

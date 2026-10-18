#!/usr/bin/env node
// Hash the final files, after all Vite/prerender/singlefile/install transformations.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { htmlContents } from './html-composition.mjs';
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const relative = (root, file) => path.relative(root, file).split(path.sep).join('/');
export function files(root) {
  return fs.readdirSync(root, { withFileTypes: true }).sort((a, b) => a.name < b.name ? -1 : 1)
    .flatMap(entry => {
      const file = path.join(root, entry.name);
      if (entry.isSymbolicLink()) throw new Error('Symlink in shipped artifact tree');
      if (entry.isDirectory()) return files(file);
      if (!entry.isFile()) throw new Error('Nonregular shipped artifact');
      return [file];
    });
}
function graph(root, filename, graphPrefix, outputPrefix = '') {
  const bytes = fs.readFileSync(path.join(root, filename));
  const value = JSON.parse(bytes);
  if (value.schema !== 'nyxid.rendered-composition.v1' || !value.chunks?.length) throw new Error('Missing rendered graph');
  const lock = fs.readFileSync(path.join(ROOT, 'frontend/package-lock.json'));
  const packages = JSON.parse(lock).packages;
  const validatePackage = identity => {
    const locked = packages[identity?.lock_path];
    const name = locked?.name ?? identity?.lock_path?.slice(identity.lock_path.lastIndexOf('node_modules/') + 'node_modules/'.length);
    if (!locked?.version || !locked.integrity || identity.name !== name || identity.version !== locked.version ||
      identity.integrity !== locked.integrity || identity.purl !== `pkg:npm/${name.replace(/^@/, '%40')}@${locked.version}`) {
      throw new Error('Stale or mislabeled module package identity');
    }
  };
  if (value.lock_sha256 !== sha256(lock)) throw new Error('Stale composition graph lock');
  for (const chunk of value.chunks) {
    if (!chunk.modules.length || /(^\/|\.\.|!~\{)/.test(chunk.path)) throw new Error('Invalid rendered output');
    if (chunk.modules.some(module => !(module.rendered_length > 0))) throw new Error('Unrendered module in shipped graph');
    chunk.path = outputPrefix + chunk.path;
  }
  if (!value.styles?.outputs || !value.styles.sources || !value.styles.generators) throw new Error('Missing style composition');
  for (const module of value.chunks.flatMap(chunk => chunk.modules).concat(value.styles.sources)) {
      if (!module.id.startsWith('repo:') && !module.id.startsWith('pkg:npm/') && !module.id.startsWith('generated:')) throw new Error('Unknown module identity');
      if (module.id.includes(ROOT) || module.id.includes('node_modules/')) throw new Error('Host path in rendered identity');
      if (module.id.startsWith('pkg:npm/') && (!module.package?.integrity || !module.package?.purl)) throw new Error('Unmapped third-party module');
      let source;
      if (module.package) {
        validatePackage(module.package);
        const prefix = module.package.purl + '/';
        if (!module.id.startsWith(prefix)) throw new Error('Module package identity mismatch');
        source = path.resolve(ROOT, 'frontend', module.package.lock_path, module.id.slice(prefix.length));
        const packageRoot = path.resolve(ROOT, 'frontend', module.package.lock_path) + path.sep;
        if (!source.startsWith(packageRoot)) throw new Error('Invalid package module path');
      } else if (module.id.startsWith('repo:')) {
        source = path.resolve(ROOT, module.id.slice(5));
        if (!source.startsWith(ROOT + path.sep)) throw new Error('Invalid repository module path');
      } else {
        const generated = module.generated_by;
        validatePackage(generated);
      }
      if (source && sha256(fs.readFileSync(source)) !== module.source_sha256) throw new Error('Stale rendered module source');
  }
  for (const generator of value.styles.generators) {
    validatePackage({ ...generator, lock_path: 'node_modules/' + generator.name });
  }
  value.styles.outputs = value.styles.outputs.map(name => {
    if (/(^\/|\.\.|!~\{)/.test(name)) throw new Error('Invalid stylesheet output');
    return outputPrefix + name;
  });
  value.inline_candidates = (value.inline_candidates ?? []).map(candidate => {
    if (!candidate.path || /(^\/|\.\.|!~\{)/.test(candidate.path)) throw new Error('Invalid inline output');
    const tool = candidate.source ? 'vite' : 'vite-plugin-singlefile';
    if (!packages['node_modules/' + tool]?.integrity || candidate.transform !== `pkg:npm/${tool}@${packages['node_modules/' + tool].version}`) {
      throw new Error('Unknown inline transform identity');
    }
    if (candidate.source) {
      const source = path.resolve(ROOT, candidate.source.slice(5));
      if (!candidate.source.startsWith('repo:') || !source.startsWith(ROOT + path.sep) ||
        !htmlContents(fs.readFileSync(source, 'utf8')).inline.some(block => block.kind === candidate.kind && block.sha256 === candidate.source_block_sha256)) {
        throw new Error('Stale inline template source');
      }
    }
    return { ...candidate, path: outputPrefix + candidate.path };
  });
  return { path: graphPrefix + filename, sha256: sha256(bytes), ...value };
}
// Exact-byte first-party static sources, never inferred third-party provenance.
function staticSources() {
  return [path.join(ROOT, 'frontend/src'), path.join(ROOT, 'frontend/public')]
    .flatMap(files).filter(file => /\.(?:js|mjs|cjs|wasm)$/.test(file))
    .concat(path.join(ROOT, 'frontend/scripts/legal-theme.js'));
}
export function seal({ root, component, output, verify = false, wizardGraph }) {
  root = path.resolve(root);
  const outputFile = path.resolve(output);
  const selected = file => component !== 'credential-accept' || /^(?:credential-accept|release-integrity)\//.test(relative(root, file));
  const inventory = files(root).filter(file => selected(file) && file !== outputFile);
  const graphs = component === 'wizard'
    ? [graph(path.dirname(wizardGraph), path.basename(wizardGraph), '', '')]
    : [
      ...(component === 'spa' ? [graph(root, 'composition-graph.json', '')] : []),
      graph(root, 'credential-accept/composition-graph.json', '', 'credential-accept/'),
    ];
  if (component === 'wizard') {
    for (const chunk of graphs[0].chunks) chunk.path = 'index.html';
    graphs[0].styles.outputs = graphs[0].styles.outputs.map(() => 'index.html');
    graphs[0].inline_candidates = graphs[0].inline_candidates.map(candidate => ({ ...candidate, path: 'index.html' }));
    graphs[0].path = 'embedded-rendered-graph';
  }
  const covered = new Set(graphs.flatMap(item => item.chunks.map(chunk => chunk.path).concat(item.styles.outputs)));
  const sourceFiles = staticSources();
  const staticProvenance = JSON.parse(fs.readFileSync(path.join(ROOT, 'frontend/static-asset-provenance.json'))).assets;
  const artifacts = inventory.map(file => {
    const name = relative(root, file);
    const bytes = fs.readFileSync(file);
    const record = { path: name, size: bytes.length, sha256: sha256(bytes) };
    if (/\.(?:js|mjs|cjs|wasm|css)$/.test(name) && !covered.has(name)) {
      const source = sourceFiles.find(candidate => fs.readFileSync(candidate).equals(bytes));
      if (!source) throw new Error('Executable asset lacks rendered or reviewed static composition: ' + name);
      record.source = 'repo:' + relative(ROOT, source);
    }
    if (/\.(?:woff2?|ttf|otf)$/.test(name)) {
      const provenance = staticProvenance.find(item => item.sha256 === record.sha256);
      if (!provenance) throw new Error('Unmapped shipped font: ' + name);
      if (sha256(fs.readFileSync(path.join(ROOT, provenance.source))) !== provenance.sha256 ||
          sha256(fs.readFileSync(path.join(ROOT, provenance.license_source))) !== provenance.license_sha256) throw new Error('Stale static provenance');
      const licenseFile = inventory.find(candidate => sha256(fs.readFileSync(candidate)) === provenance.license_sha256);
      if (!licenseFile) throw new Error('Missing shipped font license');
      record.provenance = { ...provenance, shipped_license: relative(root, licenseFile) };
    }
    return record;
  });
  for (const name of covered) if (!artifacts.some(item => item.path === name)) throw new Error('Rendered output missing from final bytes: ' + name);
  const components = [...new Map(graphs.flatMap(item => item.chunks.flatMap(chunk => chunk.modules).concat(item.styles.sources)
    .map(module => module.package ?? module.generated_by).filter(Boolean)
    .map(identity => [identity.purl, identity]))).values()]
    .sort((a, b) => a.purl < b.purl ? -1 : a.purl > b.purl ? 1 : 0);
  const remotePolicy = JSON.parse(fs.readFileSync(path.join(ROOT, 'frontend/html-resource-policy.json')));
  const inputSources = new Map();
  const inlineCandidates = graphs.flatMap(item => item.inline_candidates ?? []);
  const usedCandidates = new Set();
  const html = artifacts.filter(item => item.path.endsWith('.html')).map(item => {
    const contents = htmlContents(fs.readFileSync(path.join(root, item.path), 'utf8'));
    const template = component === 'wizard' ? 'frontend/wizard.html' : item.path === 'index.html' ? 'frontend/index.html' :
      item.path === 'credential-accept/credential-accept.html' ? 'frontend/credential-accept.html' : null;
    const allowed = [];
    if (template) {
      const bytes = fs.readFileSync(path.join(ROOT, template));
      inputSources.set(template, sha256(bytes));
      allowed.push(...htmlContents(bytes.toString()).inline.map(block => ({ ...block, source: 'repo:' + template })));
    }
    if (['privacy.html', 'terms.html'].includes(item.path)) {
      for (const source of ['frontend/scripts/legal-page.css', 'frontend/scripts/prerender-legal.mjs', 'frontend/scripts/legal-theme.js']) {
        inputSources.set(source, sha256(fs.readFileSync(path.join(ROOT, source))));
      }
      const css = fs.readFileSync(path.join(ROOT, 'frontend/scripts/legal-page.css'));
      allowed.push({ kind: 'style', sha256: sha256(css), size: css.length, source: 'repo:frontend/scripts/legal-page.css' });
    }
    const inline = contents.inline.map(block => {
      const origin = allowed.find(candidate => candidate.kind === block.kind && candidate.sha256 === block.sha256 && candidate.size === block.size);
      const generated = inlineCandidates.find(candidate => candidate.path === item.path && candidate.kind === block.kind && candidate.sha256 === block.sha256 && candidate.size === block.size);
      if (origin && !generated) return { ...block, source: origin.source };
      if (!generated) throw new Error('Unmapped inline ' + block.kind + ' in final HTML: ' + item.path);
      usedCandidates.add(generated);
      if (generated.source) {
        const source = generated.source.slice(5);
        inputSources.set(source, sha256(fs.readFileSync(path.join(ROOT, source))));
      }
      return { ...block, transform: generated.transform, ...(generated.source ? { source: generated.source, source_block_sha256: generated.source_block_sha256 } : {}) };
    });
    const references = contents.references.map(reference => {
      const url = new URL(reference.url, 'https://nyxid-build.invalid/' + item.path);
      if (url.origin !== 'https://nyxid-build.invalid') {
        if (!remotePolicy[reference.kind]?.includes(url.href)) throw new Error('Unmapped remote HTML reference');
        return { ...reference, runtime_remote: true };
      }
      const local = decodeURIComponent(url.pathname.slice(1));
      if (url.search || url.hash || !artifacts.some(artifact => artifact.path === local) ||
        (reference.kind === 'script' ? !/\.[mc]?js$/.test(local) : !local.endsWith('.css')) ||
        (!covered.has(local) && !artifacts.find(artifact => artifact.path === local)?.source)) {
        throw new Error('Unmapped local HTML reference: ' + reference.url);
      }
      return { ...reference, artifact: local };
    });
    return { path: item.path, inline, references };
  });
  if (inlineCandidates.some(candidate => !usedCandidates.has(candidate))) throw new Error('Captured singlefile code is absent from final HTML');
  const value = {
    schema: 'nyxid.shipped-composition.v1', component,
    source_lock_sha256: sha256(fs.readFileSync(path.join(ROOT, 'frontend/package-lock.json'))),
    graphs, components, artifacts, html,
    postprocess_sources: [...inputSources].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([source, digest]) => ({ id: 'repo:' + source, sha256: digest })),
  };
  const bytes = JSON.stringify(value, null, 2) + '\n';
  if (verify) {
    if (fs.readFileSync(outputFile, 'utf8') !== bytes) throw new Error('Stale shipped composition seal: ' + output);
  } else {
    fs.mkdirSync(path.dirname(outputFile), { recursive: true });
    fs.writeFileSync(outputFile, bytes);
  }
  return value;
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [component, mode = 'write'] = process.argv.slice(2);
  if (!['spa', 'credential-accept', 'wizard'].includes(component) || !['write', 'verify'].includes(mode)) throw new Error('Usage: seal-shipped-composition.mjs spa|credential-accept|wizard [write|verify]');
  const wizard = component === 'wizard';
  seal({ component, verify: mode === 'verify',
    root: path.join(ROOT, wizard ? 'cli/src/wizard/assets' : 'frontend/dist'),
    output: path.join(ROOT, wizard ? 'cli/src/wizard/assets/shipped-composition.json' : component === 'spa' ? 'frontend/dist/shipped-composition.json' : 'frontend/dist/release-integrity/credential-composition.json'),
    wizardGraph: path.join(ROOT, 'cli/src/wizard/bundle-meta/rendered-graph.json'),
  });
  console.log(component + ' final-byte composition ' + (mode === 'verify' ? 'verified' : 'sealed'));
}

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { spawn, spawnSync, execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';

const root = fileURLToPath(new URL('../', import.meta.url));
process.chdir(root);
const compass = '/usr/local/bin/compass';
const output = 'compass-out/verification';
const read = (p) => JSON.parse(fs.readFileSync(p, 'utf8'));
const hash = (b) => createHash('sha256').update(b).digest('hex');
const runJson = (args) => JSON.parse(execFileSync(compass, args, { encoding: 'utf8' }));
const write = (name, value) => fs.writeFileSync(`${output}/${name}.json`, JSON.stringify(value, null, 2) + '\n');
const git = (args) => execFileSync('git', args, { encoding: 'utf8' }).trim();
const sourceCommit = git(['rev-parse', 'HEAD']);
const configText = fs.readFileSync('.compass/config.toml', 'utf8');
// Compass init writes these scope arrays as JSON-compatible TOML string arrays.
const rules = (name) => JSON.parse(configText.match(new RegExp(`^${name} = (\\[.*\\])$`, 'm'))?.[1] ?? 'null');
const include = rules('include'), exclude = rules('exclude');
assert.ok(Array.isArray(include) && Array.isArray(exclude), 'Unsupported scope configuration');
const candidates = [...new Set(include.flatMap((pattern) => [...fs.globSync(pattern)]))]
  .filter((p) => fs.lstatSync(p).isFile() && !exclude.some((pattern) => path.matchesGlob(p, pattern)))
  .sort();
assert.ok(candidates.length, 'Source scope is empty');
const ignored = spawnSync('git', ['check-ignore', '--stdin', '-z'], {
  input: candidates.join('\0') + '\0', encoding: 'utf8',
});
if (ignored.error) throw ignored.error;
assert.ok([0, 1].includes(ignored.status), ignored.stderr);
const ignoredPaths = new Set(ignored.stdout.split('\0').filter(Boolean));
const expected = candidates.filter((p) => !ignoredPaths.has(p));
const graphBytes = fs.readFileSync('compass-out/graph.json');
const graph = JSON.parse(graphBytes);
const files = graph.graph.files.filter((f) => f.extractionStatus === 'extracted');
const actual = files.map((f) => f.path).sort();
assert.deepEqual(actual, expected, 'Extracted source scope differs from the independent file inventory');
const stale = files.filter((f) => f.contentDigest !== `sha256:${hash(fs.readFileSync(f.path))}`);
assert.equal(stale.length, 0, `Stale graph files: ${stale.map((f) => f.path)}`);
const failures = graph.graph.files.filter((f) => !['extracted', 'excluded'].includes(f.extractionStatus));
assert.equal(failures.length, 0, 'Extraction failures detected');
const crateCounts = Object.fromEntries([...new Set(actual.filter((p) => p.startsWith('crates/')).map((p) => p.split('/')[1]))].sort()
  .map((name) => [name, actual.filter((p) => p.startsWith(`crates/${name}/src/`)).length]));
const crateRoots = fs.readdirSync('crates', { withFileTypes: true })
  .filter((d) => d.isDirectory() && fs.existsSync(`crates/${d.name}/src`)).map((d) => d.name).sort();
assert.deepEqual(Object.keys(crateCounts), crateRoots, 'A crate source tree is missing');
for (const prefix of ['web/src/', 'sdks/typescript/src/', 'sdks/go/', 'sdks/flutter/lib/']) assert.ok(actual.some((p) => p.startsWith(prefix)), 'Missing source root: ' + prefix);
const quality = runJson(['diagnose', 'quality', '--graph', 'compass-out/graph.json', '--json']);
const store = runJson(['store', 'validate', 'compass-out', '--format', 'json']);
assert.equal(store.valid, true, 'Graph store validation failed');
assert.equal(quality.dangling_edges, 0);
assert.equal(quality.duplicate_node_ids, 0);
assert.equal(quality.valid_node_anchors, quality.node_count);
assert.equal(quality.valid_edge_anchors, quality.edge_count);
assert.equal(quality.output_consistency.stats_match_graph, true);
assert.equal(quality.output_consistency.stats_bytes_match_graph, true);
for (const field of ['publication_omitted_nodes', 'publication_omitted_edges', 'identity_collisions']) {
  assert.equal(quality.graph_diagnostics[field], 0, field);
}
assert.equal(quality.graph_diagnostics.by_severity.error ?? 0, 0);
const nodes = new Map(graph.nodes.map((n) => [n.id, n]));
const pairs = [
 ['flint_gate_core::auth::jwt_verify::JwtVerifyAuthenticator::new', 'flint_gate_core::auth::jwks::JwksCache::new'],
 ['flint_gate_core::auth::kratos::select_credential', 'flint_gate_core::auth::kratos::extract_cookie'],
 ['flint_gate_core::stream::processor::SseStreamProcessor::process_chunk', 'flint_gate_core::stream::processor::SseStreamProcessor::process_line'],
 ['flint_gate_core::auth::token_exchange::exchange', 'flint_gate_core::auth::token_exchange::build_delegated_claims'],
 ['flint_gate_core::authority::consumer::AuthorityConsumer::catch_up', 'flint_gate_core::authority::consumer::AuthorityConsumer::replay'],
];
const calls = pairs.map(([from, to]) => {
  const edge = graph.links.find((e) => e.kind === 'calls'
    && nodes.get(e.source)?.qualifiedName === from && nodes.get(e.target)?.qualifiedName === to);
  assert.ok(edge, `Representative call missing: ${from} -> ${to}`);
  const site = edge.relationshipSite;
  const source = fs.readFileSync(site.file);
  const expression = source.subarray(site.startByte, site.endByte).toString('utf8');
  assert.ok(expression.includes(to.split('::').at(-1)), `Call anchor does not match source: ${from}`);
  return { from, to, site, expression, edgeId: edge.id };
});

const config = read('.mcp.json').mcpServers['compass-flint-gate'];
assert.equal(config.command, compass);
assert.deepEqual(config.args, ['serve', '--graph', path.join(root, 'compass-out/graph.json'), '--transport', 'stdio']);
const child = spawn(config.command, config.args, { cwd: root, stdio: ['pipe', 'pipe', 'pipe'] });
let buffer = '', stderr = '', nextId = 0, terminalError;
const pending = new Map();
const rejectAll = (error) => {
  terminalError = error;
  for (const p of pending.values()) p.reject(error);
  pending.clear();
};
const timer = setTimeout(() => { rejectAll(Error('MCP verification timed out')); child.kill(); }, 45000);
child.on('error', rejectAll);
const closed = new Promise((resolve) => child.on('close', (code, signal) => {
  if (pending.size) rejectAll(Error(`MCP exited while requests were pending: ${code ?? signal}`));
  resolve();
}));
child.stdin.on('error', rejectAll);
child.stderr.on('data', (b) => { stderr = (stderr + b).slice(-16000); });
child.stdout.on('data', (b) => {
  buffer += b;
  let index;
  while ((index = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, index); buffer = buffer.slice(index + 1);
    if (!line.trim()) continue;
    try {
      const response = JSON.parse(line), request = pending.get(response.id);
      if (request) {
        pending.delete(response.id);
        response.error ? request.reject(Error(JSON.stringify(response.error))) : request.resolve(response.result);
      }
    } catch (error) { rejectAll(error); child.kill(); }
  }
});
function rpc(method, params = {}) {
  if (terminalError) return Promise.reject(terminalError);
  return new Promise((resolve, reject) => {
    const id = ++nextId; pending.set(id, { resolve, reject });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params: { ...params, _meta: {
      'io.modelcontextprotocol/protocolVersion': '2026-07-28',
      'io.modelcontextprotocol/clientInfo': { name: 'flint-gate-graph-verification', version: '1' },
      'io.modelcontextprotocol/clientCapabilities': {},
    } } }) + '\n');
  });
}
const receipts = [];
try {
  receipts.push({ method: 'server/discover', result: await rpc('server/discover') });
  const tools = await rpc('tools/list');
  receipts.push({ method: 'tools/list', result: tools });
  for (const required of ['graph_stats', 'search_symbols', 'get_callees']) {
    assert.ok(tools.tools.some((t) => t.name === required), `Missing MCP tool: ${required}`);
  }
  for (const [name, args, expectedText] of [
    ['graph_stats', {}, null],
    ['search_symbols', { query: 'JwtVerifyAuthenticator' }, 'JwtVerifyAuthenticator'],
    ['get_callees', { symbol: pairs[0][0] }, pairs[0][1]],
    ['get_callees', { symbol: pairs[2][0] }, pairs[2][1]],
  ]) {
    const result = await rpc('tools/call', { name, arguments: args });
    assert.ok(!result.isError, `MCP ${name} returned an error`);
    if (expectedText) assert.ok(JSON.stringify(result).includes(expectedText), `MCP ${name} omitted ${expectedText}`);
    receipts.push({ method: name, args, result });
  }
  // Catch source or graph changes during verification, before certifying this snapshot.
  assert.equal(git(['rev-parse', 'HEAD']), sourceCommit, 'HEAD changed during verification');
  assert.equal(hash(fs.readFileSync('compass-out/graph.json')), hash(graphBytes), 'Graph changed during verification');
  for (const f of files) assert.equal(`sha256:${hash(fs.readFileSync(f.path))}`, f.contentDigest, `Source changed: ${f.path}`);
  fs.mkdirSync(output, { recursive: true });
  write('scope', { sourceCommit, include, exclude, expectedFiles: expected, ignoredFiles: [...ignoredPaths], crateCounts });
  write('quality', quality);
  write('store', store);
  write('mcp', { config, receipts, stderr });
  const report = {
    checkedAt: new Date().toISOString(), sourceCommit,
    workingTreeStatus: git(['status', '--short', '--untracked-files=normal']),
    graphSha256: hash(graphBytes), build: graph.graph.build,
    fileCount: actual.length, crateCounts, files: files.map(({ path, contentDigest }) => ({ path, contentDigest })),
    missing: [], unexpected: [], stale: [], extractionFailures: [],
    quality, store, representativeCalls: calls,
    mcp: { config, discoveredTools: tools.tools.map((t) => t.name), successfulQueries: receipts.slice(2).map((r) => r.method), protocol: '2026-07-28' },
    limits: [
      'Inline test modules may remain in indexed files; counts are not production-only.',
      'Feature-gated source is not one compiled configuration.',
      'Macro expansion, dynamic dispatch, SQL and process boundaries may remain unresolved.',
      'Source-backed edges do not prove complete runtime reachability.',
      'MCP smoke verification does not register the server in the current Codex session.',
    ],
  };
  write('verification', report);
  console.log(JSON.stringify({ files: actual.length, crates: crateRoots.length, nodes: quality.node_count,
    edges: quality.edge_count, storeValid: store.valid, tools: tools.tools.length,
    successfulQueries: report.mcp.successfulQueries, report: path.join(root, output, 'verification.json') }, null, 2));
} finally {
  clearTimeout(timer);
  child.kill();
  const killTimer = setTimeout(() => child.kill('SIGKILL'), 2000);
  await closed;
  clearTimeout(killTimer);
}

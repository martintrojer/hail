import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import openapiTS, { astToString } from 'openapi-typescript';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const webappRoot = path.resolve(__dirname, '..');
const fixturePath = path.join(webappRoot, 'src', 'api', 'openapi.example.json');
const outputPath = path.join(webappRoot, 'src', 'api', 'types.ts');
const defaultOpenapiUrl = 'http://localhost:8080/api/openapi.json';
const fixtureSource = path.relative(webappRoot, fixturePath);

const usage = `Usage: npm run api:types

By default this command reads ${fixtureSource} and does not contact localhost,
so normal builds are deterministic even if an old hail-api is running.

To intentionally regenerate from a live/temp API, use one of:
  OPENAPI_SOURCE=live npm run api:types
  OPENAPI_URL=http://127.0.0.1:8080/api/openapi.json npm run api:types

Live OpenAPI documents must include every path present in ${fixtureSource};
missing fixture paths are treated as a stale API and fail the command.
`;

function shouldShowHelp() {
  return process.argv.includes('--help') || process.argv.includes('-h');
}

function requestedSource() {
  const source = process.env.OPENAPI_SOURCE;
  if (source && source !== 'fixture' && source !== 'live') {
    throw new Error(`OPENAPI_SOURCE must be "fixture" or "live", got "${source}".\n\n${usage}`);
  }

  if (process.env.OPENAPI_URL && source === 'fixture') {
    throw new Error(
      `OPENAPI_URL was set but OPENAPI_SOURCE=fixture was requested. Unset OPENAPI_URL or use OPENAPI_SOURCE=live.\n\n${usage}`,
    );
  }

  if (source) {
    return source;
  }

  return process.env.OPENAPI_URL ? 'live' : 'fixture';
}

async function readJson(filePath) {
  const contents = await fs.readFile(filePath, 'utf8');
  return JSON.parse(contents);
}

function ensureLiveSpecIncludesFixturePaths(liveSpec, fixtureSpec, openapiUrl) {
  const fixturePaths = Object.keys(fixtureSpec.paths ?? {});
  const livePaths = new Set(Object.keys(liveSpec.paths ?? {}));
  const missingFixturePaths = fixturePaths.filter((path) => !livePaths.has(path));

  if (missingFixturePaths.length > 0) {
    throw new Error(
      `${openapiUrl} appears stale; missing fixture paths: ${missingFixturePaths.join(', ')}. ` +
        `Regenerate from the checked-out hail-api or omit OPENAPI_SOURCE/OPENAPI_URL to use ${fixtureSource}.`,
    );
  }
}

async function fetchLiveSpec(openapiUrl) {
  const response = await fetch(openapiUrl, {
    headers: { accept: 'application/json' },
  });

  if (!response.ok) {
    throw new Error(`HTTP ${response.status} ${response.statusText}`);
  }

  return response.json();
}

async function loadSpec() {
  const source = requestedSource();
  const fixtureSpec = await readJson(fixturePath);

  if (source === 'fixture') {
    return {
      source: fixtureSource,
      spec: fixtureSpec,
    };
  }

  const openapiUrl = process.env.OPENAPI_URL ?? defaultOpenapiUrl;
  const liveSpec = await fetchLiveSpec(openapiUrl);
  ensureLiveSpecIncludesFixturePaths(liveSpec, fixtureSpec, openapiUrl);

  return {
    source: openapiUrl,
    spec: liveSpec,
  };
}

if (shouldShowHelp()) {
  console.log(usage.trimEnd());
  process.exit(0);
}

try {
  const loaded = await loadSpec();
  const ast = await openapiTS(loaded.spec);
  const banner = `// This file is auto-generated from ${loaded.source}.\n// Do not edit by hand; regenerate via npm run api:types.\n`;
  await fs.writeFile(outputPath, `${banner}${astToString(ast)}`);
  console.log(`[api:types] wrote ${path.relative(webappRoot, outputPath)} from ${loaded.source}`);
} catch (error) {
  console.error(`[api:types] ${error.message}`);
  process.exit(1);
}

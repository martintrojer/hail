import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import openapiTS, { astToString } from 'openapi-typescript';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const webappRoot = path.resolve(__dirname, '..');
const fixturePath = path.join(webappRoot, 'src', 'api', 'openapi.example.json');
const outputPath = path.join(webappRoot, 'src', 'api', 'types.ts');
const openapiUrl = process.env.OPENAPI_URL ?? 'http://localhost:8080/api/openapi.json';

async function readJson(filePath) {
  const contents = await fs.readFile(filePath, 'utf8');
  return JSON.parse(contents);
}

async function loadSpec() {
  try {
    const response = await fetch(openapiUrl, {
      headers: { accept: 'application/json' },
    });

    if (!response.ok) {
      throw new Error(`HTTP ${response.status} ${response.statusText}`);
    }

    return {
      source: openapiUrl,
      spec: await response.json(),
    };
  } catch (fetchError) {
    try {
      return {
        source: path.relative(webappRoot, fixturePath),
        spec: await readJson(fixturePath),
        warning: `Could not fetch ${openapiUrl}: ${fetchError.message}`,
      };
    } catch (fixtureError) {
      return {
        source: 'placeholder',
        warning: `Could not fetch ${openapiUrl}: ${fetchError.message}; could not read fixture ${path.relative(
          webappRoot,
          fixturePath,
        )}: ${fixtureError.message}`,
      };
    }
  }
}

const loaded = await loadSpec();

if (loaded.warning) {
  console.warn(`[api:types] ${loaded.warning}`);
}

if (!loaded.spec) {
  await fs.writeFile(
    outputPath,
    [
      '// Generated placeholder; regenerate via npm run api:types.',
      '// The OpenAPI URL was unreachable and no fixture was available.',
      'export type paths = Record<string, never>;',
      'export type webhooks = Record<string, never>;',
      'export type components = Record<string, never>;',
      'export type $defs = Record<string, never>;',
      'export type operations = Record<string, never>;',
      '',
    ].join('\n'),
  );
  console.warn(`[api:types] wrote placeholder ${path.relative(webappRoot, outputPath)}`);
  process.exit(0);
}

const ast = await openapiTS(loaded.spec);
const banner = `// This file is auto-generated from ${loaded.source}.\n// Do not edit by hand; regenerate via npm run api:types.\n`;
await fs.writeFile(outputPath, `${banner}${astToString(ast)}`);
console.log(`[api:types] wrote ${path.relative(webappRoot, outputPath)} from ${loaded.source}`);

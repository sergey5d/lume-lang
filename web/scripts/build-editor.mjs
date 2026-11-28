import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';
import { lumeKeywords } from './lume-keywords.mjs';

await build({
  entryPoints: [fileURLToPath(new URL('../editor.js', import.meta.url))],
  outfile: fileURLToPath(new URL('../pkg/editor.js', import.meta.url)),
  bundle: true,
  format: 'esm',
  platform: 'browser',
  target: 'es2022',
  minify: true,
  legalComments: 'eof',
  define: { LUME_KEYWORDS: JSON.stringify(lumeKeywords()) },
  logLevel: 'info',
});

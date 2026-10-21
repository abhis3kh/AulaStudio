import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const usage = 'Usage: node tools/import-layout.mjs <path-to-KB.ini> [output.json]';
const args = process.argv.slice(2);
if (args.length === 1 && ['--help', '-h'].includes(args[0])) {
  console.log(usage);
  process.exit(0);
}
if (args.length < 1 || args.length > 2 || args[0].startsWith('--')) {
  console.error(usage);
  process.exit(1);
}

try {
  const input = resolve(args[0]);
  const output = args[1]
    ? resolve(args[1])
    : fileURLToPath(new URL('../assets/keyboard-layout.json', import.meta.url));
  if (input.toLowerCase() === output.toLowerCase()) {
    throw new Error('Input and output paths must be different.');
  }
  const raw = readFileSync(input);
  if (raw[0] === 0xfe && raw[1] === 0xff) {
    throw new Error('UTF-16BE input is unsupported. Use UTF-16LE or UTF-8.');
  }
  const ini = raw.toString(raw[0] === 0xff && raw[1] === 0xfe ? 'utf16le' : 'utf8')
    .replace(/^\uFEFF/, '');
  let label = '';
  let section = '';
  const keys = [];
  for (const sourceLine of ini.split(/\r?\n/)) {
    const line = sourceLine.trim();
    if (line.startsWith('[')) {
      section = line;
      label = '';
    }
    if (section !== '[KEY]') continue;
    if (line.startsWith(';')) label = line.slice(1).trim();
    const match = line.match(/^K(\d+)\s*=\s*(.+)$/);
    if (!match) continue;
    const parts = match[2].split(',').map(value => value.trim());
    const values = parts.map(Number);
    if (parts.length !== 8 || parts.some(value => value === '') ||
        values.some(value => !Number.isSafeInteger(value))) {
      throw new Error(`Malformed geometry at K${match[1]}.`);
    }
    const [x, y, right, bottom, , vk, , matrix] = values;
    const id = Number(match[1]);
    if (!Number.isSafeInteger(id) || id < 0 || x < 0 || y < 0 ||
        right < x || bottom < y || vk < 0 || vk > 255 || matrix < 0 || matrix >= 126) {
      throw new Error(`Out-of-range geometry at K${match[1]}.`);
    }
    keys.push({ id, label, x, y, width: right - x, height: bottom - y, vk, matrix });
  }
  if (keys.length < 80 || keys.length > 126 ||
      new Set(keys.map(key => key.matrix)).size !== keys.length ||
      new Set(keys.map(key => key.id)).size !== keys.length) {
    throw new Error('Incomplete or duplicate layout.');
  }
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, `${JSON.stringify(keys, null, 2)}\n`);
  console.log(`Imported ${keys.length} unique controls to ${output}.`);
} catch (error) {
  console.error(`Layout import failed: ${error.message}`);
  process.exitCode = 1;
}

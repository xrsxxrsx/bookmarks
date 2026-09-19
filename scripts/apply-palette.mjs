// Applies a literal colour substitution map to a UTF-8 text file.
//
// Written because the Windows shell's `Set-Content` re-encodes with the system codepage,
// which corrupted a stylesheet containing non-ASCII text. Node reads and writes UTF-8
// explicitly, so the file survives.
//
// Usage: node scripts/apply-palette.mjs <file>

import { readFileSync, writeFileSync } from 'node:fs';

const file = process.argv[2];
if (!file) {
  console.error('usage: apply-palette.mjs <file>');
  process.exit(2);
}

// Old literal -> design token. Kept as an explicit table so the intent of every change is
// visible rather than buried in a regex.
const MAP = {
  // Text on a solid accent fill.
  'color: #1a1a1a': 'color: var(--on-accent)',
  // The selected row.
  'background: #26313d': 'background: var(--selected)',
  // AO3 tag chips: slightly brighter than muted so they read as reference metadata.
  'color: #b9c6d6': 'color: var(--text)',
  // Translation badge: was green, now the cool "linked" tone.
  'background: #2d3a2a': 'background: var(--accent-wash)',
  'border-color: #3f5a3a': 'border-color: var(--accent-dim)',
  'color: #a8d6a0': 'color: var(--accent-bright)',
  // Success chip.
  'background: #21362a': 'background: var(--ok-bg)',
  'border-color: #35603f': 'border-color: var(--ok-line)',
  'color: #9ad6a8': 'color: var(--ok)',
  // Warning chip: the only warm accent left, so it stands out.
  'background: #3a2c1e': 'background: var(--warn-bg)',
  'border-color: #6b4a26': 'border-color: var(--warn-line)',
  'color: #e0a768': 'color: var(--warn)',
  // Error banner and inline error.
  'background: #3a2224': 'background: var(--err-bg)',
  'color: #f0b6b6': 'color: var(--err)',
  // Success banner.
  'background: #1f3326': 'background: var(--ok-bg)',
  'color: #b6e0c2': 'color: var(--ok)',
  // Info banner.
  'background: #242c38': 'background: var(--info-bg)',
  'color: #b8c8dc': 'color: var(--text)',
};

const original = readFileSync(file, 'utf8');
let updated = original;
const applied = [];

for (const [from, to] of Object.entries(MAP)) {
  if (!updated.includes(from)) continue;
  const count = updated.split(from).length - 1;
  updated = updated.split(from).join(to);
  applied.push(`${count}x  ${from}  ->  ${to}`);
}

if (updated === original) {
  console.log('nothing to change');
  process.exit(0);
}

writeFileSync(file, updated, 'utf8');
console.log(`updated ${file}`);
for (const line of applied) console.log(`  ${line}`);

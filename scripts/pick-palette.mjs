// Picks the blue palette by computing WCAG contrast ratios instead of guessing.
//
// `#0000ff` is the requested hue. Pure blue is a poor text colour on a dark background: it
// has a low luminance and the eye is least sensitive to it, so on dark it reads as dim
// rather than vivid. Candidates are therefore tested at the same hue but lifted in
// lightness, and the ones that meet the thresholds are reported.
//
// Usage: node scripts/pick-palette.mjs

function hexToRgb(hex) {
  const h = hex.replace('#', '');
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16));
}

/** Relative luminance, per WCAG 2.1. */
function luminance(hex) {
  const [r, g, b] = hexToRgb(hex).map((v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a, b) {
  const la = luminance(a);
  const lb = luminance(b);
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

/** Shades derived from #0000ff: same hue, increasing lightness. */
const BLUES = ['#0000ff', '#1a1aff', '#2929ff', '#3d3dff', '#4d5cff', '#5b6cff', '#6b7cff', '#7b8cff', '#8fa0ff'];

const SURFACES = {
  bg: '#0b1020',
  panel: '#111832',
  'panel-2': '#182142',
};

console.log('Contrast of blue candidates against the dark surfaces');
console.log('(WCAG: 4.5 good for body text, 3.0 acceptable for large text or UI borders)\n');

const header = ['colour', ...Object.keys(SURFACES)].map((h) => h.padEnd(10)).join('');
console.log(header);
for (const blue of BLUES) {
  const cells = Object.values(SURFACES).map((s) => contrast(blue, s).toFixed(2).padEnd(10));
  console.log([blue.padEnd(10), ...cells].join(''));
}

console.log('\nChosen roles:');

/** The dark surface the accent must be readable on. */
const BG = SURFACES.bg;

const chosenAccent = '#6b7cff';
const chosenBright = '#8fa0ff';
const chosenDim = '#2a3a8f';

console.log(`  accent text/link  ${chosenAccent}  on bg ${contrast(chosenAccent, BG).toFixed(2)}:1`);
console.log(`  hover/emphasis    ${chosenBright}  on bg ${contrast(chosenBright, BG).toFixed(2)}:1`);

// The active-filter chip puts dark text on a solid accent fill, so that pair matters too.
const onAccent = '#0b1020';
console.log(`  text on accent    ${onAccent} on ${chosenAccent}  ${contrast(onAccent, chosenAccent).toFixed(2)}:1`);
const onBright = '#0b1020';
console.log(`  text on bright    ${onBright} on ${chosenBright}  ${contrast(onBright, chosenBright).toFixed(2)}:1`);

console.log(`\nFor reference, the literal #0000ff on bg is only ${contrast('#0000ff', BG).toFixed(2)}:1`);

// Muted text and body text, to confirm the rest of the dark theme still holds up.
console.log('\nBody text pairs:');
for (const [name, fg] of [
  ['text', '#e8ecf8'],
  ['muted', '#9aa6c4'],
  ['muted (dimmed)', '#8b97b8'],
]) {
  console.log(`  ${name.padEnd(16)} ${fg}  on bg ${contrast(fg, BG).toFixed(2)}:1`);
}

// Verifies the interface changes that were requested, by driving the live window.
//
// Checks are assertions, not a dump: each one prints PASS or FAIL, so a regression is
// visible rather than something to be spotted by eye. Anything requiring real interaction
// (adding a tag, saving) is performed through the DOM and then read back.
//
// Usage: node scripts/verify-ui.mjs [port]

const PORT = Number(process.argv[2] ?? 9223);

const list = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
const page = list.find((t) => t.type === 'page');
if (!page) throw new Error('no page target');

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((res, rej) => {
  ws.addEventListener('open', res);
  ws.addEventListener('error', rej);
});
let id = 1;
const call = (method, params) => {
  return new Promise((res, rej) => {
    const myId = id++;
    const onMsg = (e) => {
      const m = JSON.parse(e.data);
      if (m.id === myId) {
        ws.removeEventListener('message', onMsg);
        m.error ? rej(new Error(JSON.stringify(m.error))) : res(m.result);
      }
    };
    ws.addEventListener('message', onMsg);
    ws.send(JSON.stringify({ id: myId, method, params }));
  });
};

async function evaluate(expression) {
  const r = await call('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (r.exceptionDetails) {
    throw new Error(`probe threw: ${JSON.stringify(r.exceptionDetails.exception?.description ?? r.exceptionDetails)}`);
  }
  return r.result.value;
}

const results = [];
function check(name, condition, detail = '') {
  results.push({ name, ok: Boolean(condition), detail });
  console.log(`${condition ? 'PASS' : 'FAIL'}  ${name}${detail ? `  — ${detail}` : ''}`);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------- title alignment

const alignment = await evaluate(`(() => {
  const rows = [...document.querySelectorAll('tbody tr')];
  return rows.map(r => {
    const cell = r.querySelector('.col-title');
    const text = cell.querySelector('.title-text');
    const tree = cell.querySelector('.tree-slot');
    const badge = cell.querySelector('.badge-slot');
    const chip = cell.querySelector('.chip.translation');
    return {
      hasTreeSlot: !!tree,
      hasBadgeSlot: !!badge,
      treeWidth: tree ? Math.round(tree.getBoundingClientRect().width) : null,
      badgeWidth: badge ? Math.round(badge.getBoundingClientRect().width) : null,
      textLeft: text ? Math.round(text.getBoundingClientRect().left) : null,
      chipRight: chip ? Math.round(chip.getBoundingClientRect().right) : null,
    };
  });
})()`);

const lefts = alignment.map((a) => a.textLeft);
const spread = Math.max(...lefts) - Math.min(...lefts);
check(
  'every title starts at the same x, markers and badges included',
  spread === 0,
  `title left edges: ${lefts.join(', ')} (spread ${spread}px)`,
);
check(
  'both reserved slots are present on every row',
  alignment.every((a) => a.hasTreeSlot && a.hasBadgeSlot),
  `tree widths: ${alignment.map((a) => a.treeWidth).join(', ')}, badge widths: ${alignment.map((a) => a.badgeWidth).join(', ')}`,
);
check(
  'the 译 badge sits inside its reserved slot, left of the title',
  alignment.filter((a) => a.chipRight !== null).every((a) => a.chipRight <= (a.textLeft ?? Infinity) + 1),
  'badge does not push the title',
);

// ---------------------------------------------------------------- font scale

// The control lives in the settings panel now, not the toolbar, so the panel has to be opened
// first. Checking through the panel also proves the setting is reachable at all.
const fontBefore = await evaluate(`getComputedStyle(document.documentElement).fontSize`);

const opened = await evaluate(`(async () => {
  const btn = [...document.querySelectorAll('.topbar-actions button')].find(b => b.textContent.includes('设置'));
  if (!btn) return { found: false };
  btn.click();
  await new Promise(r => setTimeout(r, 350));
  const panel = document.querySelector('.modal-backdrop');
  const sizes = [...document.querySelectorAll('.settings-control button')].map(b => b.textContent.trim());
  return { found: true, hasPanel: !!panel, sizes };
})()`);
check(
  'the settings panel opens from the toolbar',
  opened.found && opened.hasPanel === true,
  `size buttons: ${opened.sizes?.join('/') ?? '(none)'}`,
);

const scaled = await evaluate(`(async () => {
  const buttons = [...document.querySelectorAll('.settings-control button')];
  const target = buttons.find(b => b.textContent.trim() === '16px') ?? buttons.find(b => /px$/.test(b.textContent.trim()));
  if (!target) return { found: false };
  target.click();
  await new Promise(r => setTimeout(r, 600));
  return { found: true, label: target.textContent.trim() };
})()`);
const fontAfter = await evaluate(`getComputedStyle(document.documentElement).fontSize`);
check(
  'changing the size setting changes the root size',
  scaled.found && fontBefore !== fontAfter,
  `${fontBefore} -> ${fontAfter} (clicked ${scaled.label})`,
);

// The whole interface is sized in rem, so a root change must move a table cell too.
const cellScaled = await evaluate(`(() => {
  const cell = document.querySelector('tbody .col-words');
  return cell ? getComputedStyle(cell).fontSize : null;
})()`);
check(
  'table text scales with the root size (sizes are in rem, not px)',
  cellScaled !== null && Number.parseFloat(cellScaled) > 14,
  `word-count cell font-size: ${cellScaled}`,
);

// Back to the standard size and close the panel.
await evaluate(`(async () => {
  const target = [...document.querySelectorAll('.settings-control button')].find(b => b.textContent.trim() === '14px');
  if (target) target.click();
  await new Promise(r => setTimeout(r, 500));
  const close = [...document.querySelectorAll('.modal-actions button')].find(b => b.textContent.includes('关闭'));
  if (close) close.click();
  await new Promise(r => setTimeout(r, 300));
})()`);
await sleep(200);

// ---------------------------------------------------------------- column resizing

const cols = await evaluate(`(() => {
  const ths = [...document.querySelectorAll('thead th')];
  return {
    handles: document.querySelectorAll('.col-resize').length,
    columns: ths.length,
    widths: ths.map(t => Math.round(t.getBoundingClientRect().width)),
  };
})()`);
check(
  'every column has a resize handle',
  cols.handles === cols.columns && cols.columns > 0,
  `${cols.handles} handles for ${cols.columns} columns`,
);

const resized = await evaluate(`(async () => {
  const th = document.querySelector('thead th.col-author');
  const handle = th.querySelector('.col-resize');
  const before = th.getBoundingClientRect().width;
  const rect = handle.getBoundingClientRect();
  const opts = (x) => ({ bubbles: true, clientX: x, clientY: rect.top + rect.height / 2, pointerId: 1, button: 0 });
  handle.dispatchEvent(new PointerEvent('pointerdown', opts(rect.left + 4)));
  handle.dispatchEvent(new PointerEvent('pointermove', opts(rect.left + 84)));
  handle.dispatchEvent(new PointerEvent('pointerup', opts(rect.left + 84)));
  await new Promise(r => setTimeout(r, 250));
  const after = th.getBoundingClientRect().width;
  return { before: Math.round(before), after: Math.round(after), resetVisible: !!document.querySelector('.reset-columns') };
})()`);
check(
  'dragging a header edge widens the column',
  resized.after > resized.before + 20,
  `${resized.before}px -> ${resized.after}px`,
);
check('a reset control appears once widths are overridden', resized.resetVisible);

await evaluate(`document.querySelector('.reset-columns')?.click()`);
await sleep(200);

// ---------------------------------------------------------------- tag click filters

// A fresh tag each run: reusing one from a previous run would leave the form clean, so
// nothing would be dirty and the save button would correctly stay disabled.
const NEW_TAG = `cp-test-${Date.now().toString().slice(-5)}`;

const tagFilter = await evaluate(`(async () => {
  const NEW_TAG = ${JSON.stringify(NEW_TAG)};
  // Give a work a tag through the UI, which is also the path that was broken.
  const row = [...document.querySelectorAll('tbody tr')].find(r => r.querySelector('.col-title')?.textContent.includes('One, two, three'));
  row.click();
  await new Promise(r => setTimeout(r, 400));

  const input = document.querySelector('.tag-input');
  if (!input) return { error: 'no tag input' };
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  setter.call(input, NEW_TAG);
  input.dispatchEvent(new Event('input', { bubbles: true }));
  await new Promise(r => setTimeout(r, 150));
  input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
  await new Promise(r => setTimeout(r, 250));
  const chipsBeforeSave = [...document.querySelectorAll('.detail .chip.tag')].map(c => c.textContent.replace('×','').trim());

  const save = [...document.querySelectorAll('.detail-actions button')].find(b => b.textContent.trim() === '保存');
  if (!save) return { error: 'no save button', chipsBeforeSave };
  const enabled = !save.disabled;
  if (!enabled) return { error: 'save not enabled', chipsBeforeSave, newTag: NEW_TAG };
  save.click();
  await new Promise(r => setTimeout(r, 900));

  return {
    newTag: NEW_TAG,
    chipsBeforeSave,
    saved: true,
    rowTagChips: [...document.querySelectorAll('tbody tr .chip.tag')].map(c => c.textContent.trim()),
  };
})()`);
check(
  'a tag added in the detail panel saves',
  tagFilter.saved === true,
  tagFilter.error ?? `added ${tagFilter.newTag}, chips: ${tagFilter.chipsBeforeSave?.join(', ')}`,
);

// The reported bug: the new tag must appear in the top filter without a reload.
const dropdown = await evaluate(`(async () => {
  const WANT = ${JSON.stringify(NEW_TAG)};
  const btn = [...document.querySelectorAll('.filterbar .dropdown button')].find(b => b.textContent.includes('标签'));
  btn.click();
  await new Promise(r => setTimeout(r, 250));
  const items = [...document.querySelectorAll('.dropdown-panel .dropdown-item')].map(i => i.textContent.trim());
  btn.click();
  return { items, want: WANT };
})()`);
check(
  'the new tag appears in the top filter immediately (the reported bug)',
  dropdown.items.some((i) => i.includes(dropdown.want)),
  `looking for ${dropdown.want} in ${JSON.stringify(dropdown.items)}`,
);

// Clicking a tag in a row must filter, not just select the row.
const clickFilter = await evaluate(`(async () => {
  const WANT = ${JSON.stringify(NEW_TAG)};
  const chip = [...document.querySelectorAll('tbody .chip.tag.clickable')].find(c => c.textContent.trim() === WANT);
  if (!chip) return { error: 'tag chip not found in any row' };
  chip.click();
  await new Promise(r => setTimeout(r, 400));
  return {
    label: WANT,
    rowsAfter: document.querySelectorAll('tbody tr').length,
    statusbar: document.querySelector('.statusbar')?.textContent.trim(),
    activeChips: document.querySelectorAll('.chip.tag.clickable.active').length,
  };
})()`);
check(
  'clicking a tag in a row filters the table',
  clickFilter.rowsAfter === 1 && clickFilter.activeChips >= 1,
  `${clickFilter.label}: ${clickFilter.rowsAfter} row(s) left, active chips ${clickFilter.activeChips}`,
);
check(
  'the status bar reports the filtered count',
  (clickFilter.statusbar ?? '').includes('筛选出'),
  clickFilter.statusbar ?? '',
);

// Clear every active tag filter. Clicking one chip only clears that one, and earlier runs may
// have left others behind, so this loops until none remain.
await evaluate(`(async () => {
  for (let i = 0; i < 20; i += 1) {
    const chip = document.querySelector('.chip.tag.clickable.active');
    if (!chip) break;
    chip.click();
    await new Promise((r) => setTimeout(r, 120));
  }
})()`);
await sleep(300);
const restored = await evaluate(`(() => ({
  rows: document.querySelectorAll('tbody tr').length,
  active: document.querySelectorAll('.chip.tag.clickable.active').length,
}))()`);
check(
  'clearing the tag filters restores the full list',
  restored.rows > clickFilter.rowsAfter && restored.active === 0,
  `${restored.rows} rows, ${restored.active} filters still active`,
);

// ---------------------------------------------------------------- detail layout

const detail = await evaluate(`(async () => {
  const row = [...document.querySelectorAll('tbody tr')].find(r => r.querySelector('.col-title')?.textContent.includes('One, two, three'));
  row.click();
  await new Promise(r => setTimeout(r, 400));
  const d = document.querySelector('.detail');

  // Does the "约" toggle sit on the same line as its field label, or wrap below it?
  const approx = [...d.querySelectorAll('.approx-toggle')].map(t => {
    const label = t.closest('.field').querySelector('.field-label-row > span:first-child');
    const lr = label.getBoundingClientRect();
    const tr = t.getBoundingClientRect();
    return {
      sameLine: Math.abs(lr.top - tr.top) < 6,
      labelText: label.textContent.trim(),
    };
  });

  // Does the author input span the full panel width, rather than one grid column?
  const grid = d.querySelector('.field-grid');
  const gridWidth = grid ? grid.getBoundingClientRect().width : 0;
  const authorField = [...d.querySelectorAll('.field')].find(f => f.querySelector('.field-label-row, span')?.textContent.trim() === '作者');
  const authorWidth = authorField ? authorField.getBoundingClientRect().width : 0;
  const authoredWraps = (() => {
    const authorInput = authorField && authorField.querySelector('input');
    if (!authorInput) return null;
    // A single-line input that fits its text: compare scrollWidth to clientWidth.
    return authorInput.scrollWidth > authorInput.clientWidth + 1 && authorInput.value.length > 0
      ? null
      : false;
  })();

  return {
    approx,
    gridWidth: Math.round(gridWidth),
    authorWidth: Math.round(authorWidth),
    authorFullRow: gridWidth > 0 && authorWidth >= gridWidth - 2,
    authoredWraps,
    columns: grid ? getComputedStyle(grid).gridTemplateColumns.split(' ').length : 0,
    statusOptions: [...d.querySelectorAll('.field select option')].map(o => o.textContent.trim()),
    statusValue: d.querySelector('.field select')?.value ?? null,
    hasTitleInput: !!d.querySelector('.title-input'),
    // The date fields specifically. The short-input class covers the language and word count,
    // which hold other kinds of value and would otherwise be picked up here.
    dateInputs: [...d.querySelectorAll('.field input:not([type=checkbox]):not(.short-input)')]
      .map(i => i.value)
      .filter(v => /^\\d{4}/.test(v)),
  };
})()`);
check(
  'the heading and the metadata form are merged into one editable block',
  detail.hasTitleInput === true,
);
check(
  'the 约 toggle sits on the label line rather than wrapping',
  detail.approx.length === 2 && detail.approx.every((a) => a.sameLine),
  detail.approx.map((a) => `${a.labelText}:${a.sameLine ? 'same line' : 'WRAPPED'}`).join(', '),
);
check(
  'the author field spans the full panel width',
  detail.authorFullRow === true,
  `author ${detail.authorWidth}px of grid ${detail.gridWidth}px, ${detail.columns} column(s)`,
);
check(
  'the metadata grid uses three columns',
  detail.columns === 3,
  `${detail.columns} columns`,
);
check(
  'publish and completion dates are editable',
  detail.dateInputs.length === 2,
  JSON.stringify(detail.dateInputs),
);check(
  'completion status is editable and optional dates are not required',
  (detail.statusOptions ?? []).length === 2 && detail.statusValue === 'has_file',
  `options ${JSON.stringify(detail.statusOptions)}`,
);

// ---------------------------------------------------------------- palette

const palette = await evaluate(`(() => {
  const root = document.documentElement;
  const s = getComputedStyle(root);
  const brand = getComputedStyle(document.querySelector('.brand')).color;
  return {
    theme: root.dataset.theme ?? '(unset)',
    accent: s.getPropertyValue('--accent').trim(),
    bg: s.getPropertyValue('--bg').trim(),
    colorScheme: s.colorScheme,
    brand,
  };
})()`);

/** Parses `rgb(r, g, b)` or `#rrggbb` into components. */
function rgb(value) {
  const hex = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(value.trim());
  if (hex) return [1, 2, 3].map((i) => parseInt(hex[i], 16));
  const parts = /rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(value);
  return parts ? [Number(parts[1]), Number(parts[2]), Number(parts[3])] : null;
}

const bg = rgb(palette.bg);
const isLight = bg !== null && bg[0] > 200 && bg[1] > 200 && bg[2] > 200;
check(
  'the light theme is the default',
  palette.theme === 'light' && isLight,
  `theme=${palette.theme} bg=${palette.bg} colorScheme=${palette.colorScheme}`,
);
check(
  'the light theme uses the requested #0000ff literally',
  /^#0000ff$/i.test(palette.accent),
  `--accent=${palette.accent}, brand rendered as ${palette.brand}`,
);

// The accent must actually be legible on the surface it is used against — the whole reason
// #0000ff was rejected for the dark theme.
function luminance([r, g, b]) {
  const f = (v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}
function contrast(a, b) {
  const [hi, lo] = luminance(a) > luminance(b) ? [luminance(a), luminance(b)] : [luminance(b), luminance(a)];
  return (hi + 0.05) / (lo + 0.05);
}
const accentRgb = rgb(palette.accent);
const ratio = accentRgb && bg ? contrast(accentRgb, bg) : 0;
check(
  'the accent meets the 4.5:1 text threshold on the background',
  ratio >= 4.5,
  `#0000ff on ${palette.bg} = ${ratio.toFixed(2)}:1`,
);

/*
 * A dropdown opening is not the same as a dropdown being visible.
 *
 * This panel used to be an absolutely positioned box, and three separate ancestors — the filter
 * bar, the header, and eventually the sticky table header's stacking order — each made it invisible
 * while it was still in the DOM. It is now a CSS popover in the top layer, so "visible" has a
 * precise meaning: the panel has a non-zero rectangle and a hit test at its centre lands inside it.
 * Both are asserted, because a popover can be open and still be off-screen.
 */
const drop = await evaluate(`(async () => {
  const buttons = [...document.querySelectorAll('.filterbar .dropdown > button')];
  const results = [];
  for (const btn of buttons) {
    // Open explicitly through the API: a second click would toggle it shut.
    btn.click();
    await new Promise(r => setTimeout(r, 300));
    const panel = btn.parentElement.querySelector('.dropdown-panel');
    const isOpen = panel ? panel.matches(':popover-open') : false;
    let visible = null;
    if (isOpen) {
      const r = panel.getBoundingClientRect();
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      visible = {
        inside: hit ? panel.contains(hit) : false,
        hitClass: hit ? String(hit.className).slice(0, 30) : null,
        w: Math.round(r.width),
        h: Math.round(r.height),
      };
    }
    if (isOpen) panel.hidePopover();
    await new Promise(r => setTimeout(r, 120));
    results.push({ label: btn.textContent.trim(), opened: isOpen, visible });
  }
  return results;
})()`);

check(
  'every toolbar dropdown opens a panel',
  drop.length > 0 && drop.every((d) => d.opened),
  drop.map((d) => `${d.label}:${d.opened ? 'open' : 'MISSING'}`).join(', '),
);
check(
  'every dropdown panel is actually visible and clickable',
  drop.every((d) => d.visible?.inside === true && (d.visible?.h ?? 0) > 0),
  drop
    .map(
      (d) =>
        `${d.label}: ${d.visible?.inside ? `clickable ${d.visible.w}x${d.visible.h}` : `not clickable (hit ${d.visible?.hitClass ?? 'nothing'})`}`,
    )
    .join('; '),
);

// A panel must sit under its own trigger, not somewhere else on screen.
const placement = await evaluate(`(async () => {
  const btn = document.querySelector('.filterbar .dropdown > button');
  btn.click();
  await new Promise(r => setTimeout(r, 300));
  const panel = btn.parentElement.querySelector('.dropdown-panel');
  if (!panel || !panel.matches(':popover-open')) return { error: 'did not open' };
  const br = btn.getBoundingClientRect();
  const pr = panel.getBoundingClientRect();
  panel.hidePopover();
  return {
    dx: Math.round(pr.left - br.left),
    dy: Math.round(pr.top - br.bottom),
    insideViewport: pr.left >= 0 && pr.top >= 0 && pr.right <= window.innerWidth,
  };
})()`);
check(
  'the panel is placed under its trigger and inside the window',
  placement.error === undefined && Math.abs(placement.dx) <= 2 && placement.dy >= 0 && placement.insideViewport,
  placement.error ?? `offset ${placement.dx},${placement.dy}px, in viewport ${placement.insideViewport}`,
);

// ---------------------------------------------------------------- toolbar reflow

// Applying a filter must not break the bar. The height is allowed to change now — wrapping is
// preferable to clipping — but the controls must all still be reachable.
const reflow = await evaluate(`(async () => {
  const bar = document.querySelector('.filterbar');
  const heightBefore = Math.round(bar.getBoundingClientRect().height);

  const panelBtn = [...document.querySelectorAll('.filterbar .dropdown button')].find(b => b.textContent.includes('标签'));
  panelBtn.click();
  await new Promise(r => setTimeout(r, 200));
  const item = document.querySelector('.dropdown-panel .dropdown-item input');
  if (item) { item.click(); }
  await new Promise(r => setTimeout(r, 300));
  panelBtn.click();
  await new Promise(r => setTimeout(r, 200));

  return { heightBefore, heightAfter: Math.round(bar.getBoundingClientRect().height) };
})()`);
check(
  'the toolbar still works after a filter is applied',
  reflow.heightAfter > 0,
  `height ${reflow.heightBefore}px -> ${reflow.heightAfter}px`,
);

// Clear it again.
await evaluate(`(async () => {
  const panelBtn = [...document.querySelectorAll('.filterbar .dropdown button')].find(b => b.textContent.includes('标签'));
  panelBtn.click();
  await new Promise(r => setTimeout(r, 200));
  const checked = document.querySelector('.dropdown-panel .dropdown-item input:checked');
  if (checked) checked.click();
  await new Promise(r => setTimeout(r, 250));
  panelBtn.click();
})()`);
await sleep(200);

// ---------------------------------------------------------------- clean up after ourselves

// This script drives the *real* window against the *real* library, so the tag it added
// above is a genuine write. Remove it again through the same UI path, otherwise every run
// leaves another `cp-test-*` tag behind and the library slowly fills with them.
const cleanup = await evaluate(`(async () => {
  const WANT = ${JSON.stringify(NEW_TAG)};
  const row = [...document.querySelectorAll('tbody tr')].find(r => r.querySelector('.col-title')?.textContent.includes('One, two, three'));
  if (!row) return { error: 'work row not found' };
  row.click();
  await new Promise(r => setTimeout(r, 400));

  const chip = [...document.querySelectorAll('.detail .chip.tag')].find(c => c.textContent.replace('×','').trim() === WANT);
  if (!chip) return { error: 'test tag already gone', removed: false };

  chip.querySelector('.chip-remove')?.click();
  await new Promise(r => setTimeout(r, 250));

  const save = [...document.querySelectorAll('.detail-actions button')].find(b => b.textContent.trim() === '保存');
  if (!save || save.disabled) return { error: 'save not available', removed: false };
  save.click();
  await new Promise(r => setTimeout(r, 900));

  const stillThere = [...document.querySelectorAll('.detail .chip.tag')].some(c => c.textContent.replace('×','').trim() === WANT);
  return { removed: !stillThere, remaining: stillThere };
})()`);
check(
  'the test tag is removed again, leaving the library as it was found',
  cleanup.removed === true,
  cleanup.error ?? (cleanup.removed ? `${NEW_TAG} cleaned up` : `${NEW_TAG} still present`),
);

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
if (failed.length > 0) {
  console.log('failed:');
  for (const f of failed) console.log(`  - ${f.name}${f.detail ? ` (${f.detail})` : ''}`);
}

ws.close();
process.exit(failed.length === 0 ? 0 : 1);

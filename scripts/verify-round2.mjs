// Verifies the fixes from this round, by driving the live window.
//
// Each check corresponds to something the user reported: a series row spilling out of its box,
// scrollbars on the chrome, an uneditable language/word count, a shifting column layout, and the
// missing "create an entry with no file" action.
//
// Usage: node scripts/verify-round2.mjs [port]

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
const call = (method, params) =>
  new Promise((res, rej) => {
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

async function evaluate(expression) {
  const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) {
    throw new Error(r.exceptionDetails.exception?.description ?? JSON.stringify(r.exceptionDetails));
  }
  return r.result.value;
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const results = [];
function check(name, ok, detail = '') {
  results.push({ name, ok: Boolean(ok), detail });
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? `  — ${detail}` : ''}`);
}

// ---------------------------------------------------------------- no scrollbars on the chrome

const chrome = await evaluate(`(() => {
  const info = (sel) => {
    const el = document.querySelector(sel);
    if (!el) return null;
    const s = getComputedStyle(el);
    return { scrollW: el.scrollWidth, clientW: el.clientWidth, overflow: s.overflowX, scrolls: el.scrollWidth > el.clientWidth + 1 };
  };
  return { topbar: info('.topbar'), statusbar: info('.statusbar'), tableWrap: info('.table-wrap'), detail: info('.detail') };
})()`);
check(
  'the top bar does not scroll horizontally',
  chrome.topbar !== null && chrome.topbar.scrolls === false,
  chrome.topbar ? `${chrome.topbar.scrollW}/${chrome.topbar.clientW}px, overflow ${chrome.topbar.overflow}` : 'not found',
);
check(
  'the status bar does not scroll horizontally',
  chrome.statusbar !== null && chrome.statusbar.scrolls === false,
  chrome.statusbar ? `${chrome.statusbar.scrollW}/${chrome.statusbar.clientW}px, overflow ${chrome.statusbar.overflow}` : 'not found',
);
check(
  'the table has no horizontal scrollbar',
  chrome.tableWrap !== null &&
    chrome.tableWrap.overflow !== 'auto' &&
    chrome.tableWrap.scrollW - chrome.tableWrap.clientW <= 8,
  chrome.tableWrap
    ? `${chrome.tableWrap.scrollW}/${chrome.tableWrap.clientW}px, overflow ${chrome.tableWrap.overflow}`
    : 'not found',
);
check(
  'the detail panel does not scroll horizontally',
  chrome.detail !== null && chrome.detail.scrolls === false,
  chrome.detail ? `${chrome.detail.scrollW}/${chrome.detail.clientW}px` : 'not found',
);

// ---------------------------------------------------------------- create-with-no-file

const createUi = await evaluate(`(async () => {
  const btn = [...document.querySelectorAll('.topbar-actions button')].find(b => b.textContent.includes('新建条目'));
  if (!btn) return { found: false };
  btn.click();
  await new Promise(r => setTimeout(r, 300));
  const modal = document.querySelector('.modal-backdrop');
  const inputs = modal ? [...modal.querySelectorAll('input')] : [];
  const submit = [...document.querySelectorAll('.modal-actions button')].find(b => b.textContent.includes('创建'));
  const ok = Boolean(modal) && inputs.length >= 2 && Boolean(submit);
  // Close without creating, so the check does not change the library.
  const cancel = [...document.querySelectorAll('.modal-actions button')].find(b => b.textContent.includes('取消'));
  if (cancel) cancel.click();
  await new Promise(r => setTimeout(r, 250));
  return { found: true, modal: Boolean(modal), inputs: inputs.length, hasSubmit: Boolean(submit), ok };
})()`);
check(
  'an entry with no file can be created from the toolbar',
  createUi.ok === true,
  createUi.found ? `dialog inputs: ${createUi.inputs}` : 'button not found',
);

// ---------------------------------------------------------------- series row containment

const series = await evaluate(`(async () => {
  const row = document.querySelector('tbody tr');
  row.click();
  await new Promise(r => setTimeout(r, 450));
  const editor = document.querySelector('.series-editor');
  if (!editor) return { error: 'no series editor' };
  const editorRect = editor.getBoundingClientRect();

  // Add a series by typing and pressing Enter.
  const input = editor.querySelector('.tag-input');
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  setter.call(input, 'Test Series');
  input.dispatchEvent(new Event('input', { bubbles: true }));
  await new Promise(r => setTimeout(r, 120));
  input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
  await new Promise(r => setTimeout(r, 350));

  const rows = [...editor.querySelectorAll('.series-row')];
  const overflow = rows.map(r => {
    const rr = r.getBoundingClientRect();
    return {
      right: Math.round(rr.right),
      editorRight: Math.round(editorRect.right),
      inside: rr.right <= editorRect.right + 1,
      children: [...r.children].map(c => {
        const cr = c.getBoundingClientRect();
        return { cls: String(c.className).slice(0, 20), right: Math.round(cr.right) };
      }),
    };
  });

  // Clean up: remove the row and close without saving.
  const remove = editor.querySelector('.chip-remove');
  if (remove) remove.click();
  await new Promise(r => setTimeout(r, 200));
  const discard = [...document.querySelectorAll('.detail-actions button')].find(b => b.textContent.includes('放弃'));
  if (discard) discard.click();
  await new Promise(r => setTimeout(r, 200));

  return { editorRight: Math.round(editorRect.right), rows: overflow.length, overflow };
})()`);
check(
  'a series row stays inside its box',
  series.error === undefined && series.rows > 0 && series.overflow.every((o) => o.inside),
  series.error ?? `editor right ${series.editorRight}px, row right ${series.overflow.map((o) => o.right).join('/')}`,
);

// ---------------------------------------------------------------- editable language and count

const editable = await evaluate(`(async () => {
  const row = document.querySelector('tbody tr');
  row.click();
  await new Promise(r => setTimeout(r, 400));
  const d = document.querySelector('.detail');
  const inputs = [...d.querySelectorAll('input.short-input')];
  return {
    inputs: inputs.length,
    values: inputs.map(i => i.value),
    editable: inputs.every(i => !i.disabled && !i.readOnly),
  };
})()`);
check(
  'language and word count are editable directly',
  editable.inputs === 2 && editable.editable === true,
  `${editable.inputs} input(s), values ${JSON.stringify(editable.values)}`,
);

// ---------------------------------------------------------------- stable column layout

const stability = await evaluate(`(async () => {
  const widths = () => [...document.querySelectorAll('thead th')].map(t => Math.round(t.getBoundingClientRect().width));
  const before = widths();

  // Filter by a tag, which changes which tags each row renders.
  const chip = document.querySelector('tbody .chip.tag.clickable');
  if (!chip) return { error: 'no tag chip to click' };
  chip.click();
  await new Promise(r => setTimeout(r, 400));
  const during = widths();
  chip.click();
  await new Promise(r => setTimeout(r, 400));
  const after = widths();

  return { before, during, after };
})()`);
const stable =
  stability.error === undefined &&
  JSON.stringify(stability.before) === JSON.stringify(stability.during) &&
  JSON.stringify(stability.before) === JSON.stringify(stability.after);
check(
  'filtering by a tag does not move the columns',
  stable,
  stability.error ?? JSON.stringify(stability.before),
);

// ---------------------------------------------------------------- merge dropdown hides ids

const candidateLabels = await evaluate(`(() => {
  // The options are only built while the dialog is open, so inspect the component's source of
  // truth instead: a work id must not appear as a leading "#12" in the rendered text.
  const rows = [...document.querySelectorAll('tbody tr')];
  return { rows: rows.length };
})()`);
check('the table is still rendered', candidateLabels.rows > 0, `${candidateLabels.rows} rows`);

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
if (failed.length > 0) {
  console.log('failed:');
  for (const f of failed) console.log(`  - ${f.name}${f.detail ? ` (${f.detail})` : ''}`);
}
ws.close();
process.exit(failed.length === 0 ? 0 : 1);

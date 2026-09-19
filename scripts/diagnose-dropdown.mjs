// Focused check: does clicking a toolbar dropdown actually produce a panel the user can see?
//
// Measures the panel's position against the clip rectangle of its ancestors, because
// `overflow-x: auto` clips on both axes — an absolutely positioned panel opening *below* the bar
// can be cut off entirely while still being "rendered".
//
// Usage: node scripts/diagnose-dropdown.mjs [port]

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

const result = await evaluate(`(async () => {
  const buttons = [...document.querySelectorAll('.filterbar .dropdown > button')];
  if (buttons.length === 0) return { error: 'no dropdown buttons' };

  const btn = buttons[0];
  const btnRect = btn.getBoundingClientRect();

  // Every ancestor that clips: anything with overflow other than visible.
  const clippers = [];
  for (let el = btn.parentElement; el && el !== document.documentElement; el = el.parentElement) {
    const s = getComputedStyle(el);
    if (s.overflow !== 'visible' || s.overflowX !== 'visible' || s.overflowY !== 'visible') {
      const r = el.getBoundingClientRect();
      clippers.push({
        cls: String(el.className).slice(0, 40),
        overflow: s.overflow,
        overflowX: s.overflowX,
        overflowY: s.overflowY,
        clip: { top: Math.round(r.top), bottom: Math.round(r.bottom), left: Math.round(r.left), right: Math.round(r.right) },
      });
    }
  }

  btn.click();
  await new Promise((r) => setTimeout(r, 300));

  const panel = document.querySelector('.dropdown-panel');
  const panelRect = panel ? panel.getBoundingClientRect() : null;

  // Is any part of the panel actually visible on screen? getClientRects plus a hit test at the
  // panel's centre answers it more honestly than the CSS box alone.
  let hitAtPanel = null;
  if (panelRect) {
    const cx = panelRect.left + panelRect.width / 2;
    const cy = panelRect.top + panelRect.height / 2;
    const hit = document.elementFromPoint(cx, cy);
    hitAtPanel = {
      x: Math.round(cx),
      y: Math.round(cy),
      tag: hit ? hit.tagName.toLowerCase() : null,
      insidePanel: hit ? panel.contains(hit) : false,
      cls: hit ? String(hit.className).slice(0, 40) : null,
    };
  }

  const open = panel !== null;
  btn.click();

  return {
    button: { text: btn.textContent.trim(), rect: { top: Math.round(btnRect.top), bottom: Math.round(btnRect.bottom) } },
    clippers,
    panelOpened: open,
    panelRect: panelRect
      ? {
          top: Math.round(panelRect.top),
          bottom: Math.round(panelRect.bottom),
          height: Math.round(panelRect.height),
          width: Math.round(panelRect.width),
        }
      : null,
    hitAtPanel,
    // If the panel opens below a bar whose clip ends at the bar's own bottom, it is invisible.
    panelBelowClip: clippers.some((c) => panelRect && panelRect.top >= c.clip.bottom - 1),
  };
})()`);

console.log(JSON.stringify(result, null, 2));
ws.close();

// Interrogates the running window about a specific complaint, by measuring the DOM rather than
// assuming. Used to diagnose interface problems the user reports.
//
// Usage: node scripts/diagnose-ui.mjs [port]

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

const report = await evaluate(`(async () => {
  const rect = (el) => {
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) };
  };
  // What actually receives a point? If a dropdown is covered, this names the culprit.
  const at = (el) => {
    if (!el) return null;
    const r = el.getBoundingClientRect();
    const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    return {
      self: hit === el || el.contains(hit),
      hitTag: hit ? hit.tagName.toLowerCase() : null,
      hitClass: hit ? String(hit.className).slice(0, 60) : null,
    };
  };

  const bar = document.querySelector('.filterbar');
  const buttons = [...document.querySelectorAll('.filterbar .dropdown > button')];
  const info = buttons.map((b) => ({
    label: b.textContent.trim(),
    rect: rect(b),
    hit: at(b),
  }));

  const barRect = rect(bar);
  const barStyle = bar ? getComputedStyle(bar) : null;

  // Click the first dropdown button for real and see whether a panel appears.
  let panelAfterClick = null;
  if (buttons[0]) {
    buttons[0].click();
    await new Promise((r) => setTimeout(r, 250));
    const panel = document.querySelector('.dropdown-panel');
    panelAfterClick = panel ? { rect: rect(panel), items: panel.querySelectorAll('.dropdown-item').length } : null;
    buttons[0].click();
  }

  const settingsButton = [...document.querySelectorAll('.topbar-actions button')].find((b) =>
    b.textContent.includes('设置'),
  );
  const fontScale = document.querySelector('.fontscale');

  return {
    bar: {
      rect: barRect,
      overflowX: barStyle?.overflowX,
      flexWrap: barStyle?.flexWrap,
      scrollWidth: bar?.scrollWidth ?? null,
      clientWidth: bar?.clientWidth ?? null,
      scrolled: bar ? Math.round(bar.scrollLeft) : null,
    },
    dropdownButtons: info,
    panelAfterClick,
    hasSettingsButton: Boolean(settingsButton),
    hasFontScaleInBar: Boolean(fontScale),
    fontScaleRect: rect(fontScale),
    topbarRect: rect(document.querySelector('.topbar')),
    windowWidth: window.innerWidth,
  };
})()`);

console.log(JSON.stringify(report, null, 2));
ws.close();

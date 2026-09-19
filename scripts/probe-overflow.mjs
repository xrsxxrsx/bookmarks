// Finds which descendant is wider than its container, by walking the tree and comparing geometry.
// Guessing at overflow has been unproductive; this measures it.
//
// Usage: node scripts/probe-overflow.mjs [port] [selector]
const PORT = Number(process.argv[2] ?? 9223);
const ROOT_SELECTOR = process.argv[3] ?? '.table-wrap';

const list = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
const page = list.find((t) => t.type === 'page');
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

const lines = [
  '(() => {',
  `  const root = document.querySelector(${JSON.stringify(ROOT_SELECTOR)});`,
  '  if (!root) return { error: "no root" };',
  '  const rootRect = root.getBoundingClientRect();',
  '  const offenders = [];',
  '  const walk = (el) => {',
  '    const r = el.getBoundingClientRect();',
  '    const s = getComputedStyle(el);',
  '    if (r.right > rootRect.right + 1 || r.left < rootRect.left - 1) {',
  '      offenders.push({',
  '        tag: el.tagName.toLowerCase(),',
  '        cls: String(el.className).slice(0, 30),',
  '        left: Math.round(r.left),',
  '        right: Math.round(r.right),',
  '        w: Math.round(r.width),',
  '        pos: s.position,',
  '        display: s.display,',
  '      });',
  '    }',
  '    for (const child of el.children) walk(child);',
  '  };',
  '  for (const child of root.children) walk(child);',
  '  return {',
  '    root: { left: Math.round(rootRect.left), right: Math.round(rootRect.right), client: root.clientWidth, scroll: root.scrollWidth },',
  '    offenderCount: offenders.length,',
  '    offenders: offenders.slice(0, 14),',
  '  };',
  '})()',
];

const r = await call('Runtime.evaluate', {
  expression: lines.join('\n'),
  returnByValue: true,
});
if (r.exceptionDetails) {
  console.error('threw:', r.exceptionDetails.exception?.description ?? JSON.stringify(r.exceptionDetails));
} else {
  console.log(JSON.stringify(r.result.value, null, 2));
}
ws.close();

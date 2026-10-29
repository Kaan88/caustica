/* CDP driver for headless verification using Node's built-in fetch + WebSocket.
   Usage: node verify.mjs [--expr "<js>"] [--shot out.png] [--wait ms]           */
import fs from 'node:fs';

const PORT = process.env.CDP_PORT || '9333';
const WANT = process.env.CDP_URL || 'cornell';
const args = process.argv.slice(2);
const getOpt = (name) => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : null; };

async function findPage(retries = 30) {
  for (let i = 0; i < retries; i++) {
    try {
      const list = await fetch(`http://127.0.0.1:${PORT}/json/list`).then(r => r.json());
      const page = list.find(t => t.type === 'page' && t.webSocketDebuggerUrl && t.url.includes(WANT));
      if (page) return page;
    } catch { /* browser not up yet */ }
    await new Promise(r => setTimeout(r, 400));
  }
  throw new Error('no CDP page target matching "' + WANT + '" on port ' + PORT);
}

const page = await findPage();
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = e => rej(new Error('ws error: ' + e.message)); });

let msgId = 0;
const pending = new Map();
const logs = [];
ws.onmessage = ev => {
  const m = JSON.parse(ev.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); return; }
  if (m.method === 'Runtime.consoleAPICalled') {
    const a = (m.params.args || []).map(x => x.value ?? x.description ?? x.type).join(' ');
    logs.push('[console.' + m.params.type + '] ' + a);
  } else if (m.method === 'Runtime.exceptionThrown') {
    const d = m.params.exceptionDetails;
    logs.push('[exception] ' + (d.exception?.description || d.text));
  } else if (m.method === 'Log.entryAdded') {
    logs.push('[log.' + m.params.entry.level + '] ' + m.params.entry.text +
              (m.params.entry.sourceURL ? ' @ ' + m.params.entry.sourceURL : ''));
  }
};
const send = (method, params = {}) => new Promise((res, rej) => {
  const id = ++msgId;
  pending.set(id, res);
  ws.send(JSON.stringify({ id, method, params }));
  setTimeout(() => { if (pending.has(id)) { pending.delete(id); rej(new Error('timeout: ' + method)); } }, 60000);
});
const evl = async expr => {
  const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (process.env.CDP_DEBUG) console.log('RAW ' + JSON.stringify(r).slice(0, 2000));
  if (r.exceptionDetails) {
    return { error: r.exceptionDetails.exception?.description || r.exceptionDetails.text };
  }
  const res = r.result?.result ?? r.result;          // CDP: { result: { result: {...} } }
  return { value: res?.value, type: res?.type };
};

await send('Runtime.enable');
await send('Log.enable');
await send('Page.enable');

const wait = parseInt(getOpt('--wait') || '2500', 10);
if (args.includes('--reload')) { await send('Page.reload', { ignoreCache: true }); }
await new Promise(r => setTimeout(r, wait));

const expr = getOpt('--file') ? fs.readFileSync(getOpt('--file'), 'utf8') : getOpt('--expr');
if (expr) {
  const out = await evl(expr);
  console.log('EVAL ' + JSON.stringify(out).slice(0, 6000));
}

const shot = getOpt('--shot');
if (shot) {
  const r = await send('Page.captureScreenshot', { format: 'png' });
  if (r.result?.data) fs.writeFileSync(shot, Buffer.from(r.result.data, 'base64'));
  console.log('SCREENSHOT ' + shot);
}

if (logs.length) console.log('--- browser logs ---\n' + logs.slice(0, 40).join('\n'));
else console.log('--- no console output / exceptions ---');
ws.close();
process.exit(0);

import fs from 'node:fs';
const PORT = '9333';
const v = await fetch(`http://127.0.0.1:${PORT}/json/version`).then(r => r.json()).catch(() => null);
if (!v) { console.log('already closed'); process.exit(0); }
const ws = new WebSocket(v.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = e => rej(new Error('ws')); });
ws.send(JSON.stringify({ id: 1, method: 'Browser.close' }));
await new Promise(r => setTimeout(r, 1500));
ws.close();
console.log('Browser.close sent');
process.exit(0);

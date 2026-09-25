document.querySelector('#api').textContent = JSON.stringify(await (await fetch('/api/headers')).json());
const ws = new WebSocket(`ws://${location.host}/api/ws`);
ws.onopen = () => ws.send('browser echo');
ws.onmessage = event => document.querySelector('#ws').textContent = `WebSocket: ${event.data}`;
const stream = new EventSource('/api/stream');
stream.onmessage = event => { document.querySelector('#stream').textContent += ` | ${event.data}`; if(event.data === 'second') stream.close(); };

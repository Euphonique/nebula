const path = require('path');
const rl = require('readline').createInterface({ input: process.stdin });
const send = (m) => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', ...m }) + '\n');
let next = 100, waiting = {}, cwd = process.cwd();
const ask = (method, params) => new Promise((res) => { const id = next++; waiting[id] = res; send({ id, method, params }); });
const update = (u) => send({ method: 'session/update', params: { sessionId: 's1', update: u } });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
rl.on('line', async (line) => {
  const m = JSON.parse(line);
  if (m.id !== undefined && !m.method) { waiting[m.id](m); return; }
  if (m.method === 'initialize') send({ id: m.id, result: { protocolVersion: 1, agentCapabilities: { promptCapabilities: { embeddedContext: true } }, agentInfo: { name: 'fake', title: 'Test-Agent', version: '0.1' } } });
  if (m.method === 'session/new') { cwd = m.params.cwd; send({ id: m.id, result: { sessionId: 's1', modes: { currentModeId: 'default', availableModes: [{ id: 'default', name: 'Immer fragen' }, { id: 'acceptEdits', name: 'Änderungen annehmen' }] } } }); }
  if (m.method === 'session/prompt' && m.params.prompt[0].text === 'echo') {
    const lines = m.params.prompt.slice(1).map((b) => '- ' + b.type + ': ' + (b.resource ? b.resource.uri + ' (' + b.resource.text.length + ' Zeichen)' : (b.uri || b.text.slice(0, 60))));
    update({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Empfangen:\n\n' + lines.join('\n') } });
    send({ id: m.id, result: { stopReason: 'end_turn' } });
    return;
  }
  if (m.method === 'session/prompt') {
    const file = path.join(cwd, 'notiz.txt');
    update({ sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text: 'Ich sollte die Datei lesen.' } });
    update({ sessionUpdate: 'plan', entries: [{ content: 'notiz.txt lesen', priority: 'high', status: 'completed' }, { content: 'Zeile zwei ändern', priority: 'high', status: 'in_progress' }] });
    update({ sessionUpdate: 'tool_call', toolCallId: 'r1', title: 'Read notiz.txt', kind: 'read', status: 'in_progress', locations: [{ path: file, line: 1 }] });
    const read = await ask('fs/read_text_file', { sessionId: 's1', path: file });
    update({ sessionUpdate: 'tool_call_update', toolCallId: 'r1', status: 'completed', content: [{ type: 'content', content: { type: 'text', text: read.result.content } }] });
    for (const w of ['Ich habe ', 'die Datei **gelesen**. ', 'Hier ist mein Vorschlag:\n\n', '- Zeile zwei wird `ersetzt`\n- Rest bleibt\n\n```js\nconst x = 1;\n```\n']) { update({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: w } }); await sleep(150); }
    const neu = read.result.content.replace('Zeile zwei', 'Zeile ZWEI (vom Agenten)');
    update({ sessionUpdate: 'tool_call', toolCallId: 'e1', title: 'Edit notiz.txt', kind: 'edit', status: 'pending', locations: [{ path: file, line: 2 }], content: [{ type: 'diff', path: file, oldText: read.result.content, newText: neu }] });
    const perm = await ask('session/request_permission', { sessionId: 's1', toolCall: { toolCallId: 'e1' },
      options: [{ optionId: 'allow', name: 'Erlauben', kind: 'allow_once' }, { optionId: 'always', name: 'Immer erlauben', kind: 'allow_always' }, { optionId: 'reject', name: 'Ablehnen', kind: 'reject_once' }] });
    const o = perm.result.outcome;
    if (o.outcome === 'selected' && o.optionId !== 'reject') {
      await ask('fs/write_text_file', { sessionId: 's1', path: file, content: neu });
      update({ sessionUpdate: 'tool_call_update', toolCallId: 'e1', status: 'completed' });
      update({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Fertig – die Datei ist geändert.' } });
    } else {
      update({ sessionUpdate: 'tool_call_update', toolCallId: 'e1', status: 'failed' });
    }
    send({ id: m.id, result: { stopReason: 'end_turn' } });
  }
});

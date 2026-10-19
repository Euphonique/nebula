//! The session against a scripted fake agent (Node), covering the handshake,
//! streamed text, tool calls with diffs, permission questions and file access.

use std::time::{Duration, Instant};

use super::AgentConfig;
use super::session::{AgentSession, Entry, FileRequest, Phase};

const FAKE_AGENT: &str = r#"
const rl = require('readline').createInterface({ input: process.stdin });
const send = (m) => process.stdout.write(JSON.stringify({ jsonrpc: '2.0', ...m }) + '\n');
let next = 100, waiting = {}, promptId = null;
const ask = (method, params) => new Promise((res) => { const id = next++; waiting[id] = res; send({ id, method, params }); });
const update = (u) => send({ method: 'session/update', params: { sessionId: 's1', update: u } });
rl.on('line', async (line) => {
  const m = JSON.parse(line);
  if (m.id !== undefined && !m.method) { waiting[m.id](m); return; }
  if (m.method === 'initialize') send({ id: m.id, result: { protocolVersion: 1, agentCapabilities: {}, agentInfo: { name: 'fake', version: '1.0' } } });
  if (m.method === 'session/new') send({ id: m.id, result: { sessionId: 's1', modes: { currentModeId: 'default', availableModes: [{ id: 'default', name: 'Standard' }] } } });
  if (m.method === 'session/cancel') send({ id: promptId, result: { stopReason: 'cancelled' } });
  if (m.method === 'session/prompt') {
    promptId = m.id;
    if (m.params.prompt[0].text === 'warte') return;
    update({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Hallo ' } });
    update({ sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Welt' } });
    update({ sessionUpdate: 'plan', entries: [{ content: 'Lesen', priority: 'high', status: 'completed' }, { content: 'Ändern', priority: 'high', status: 'in_progress' }] });
    const read = await ask('fs/read_text_file', { sessionId: 's1', path: 'C:\\x\\a.txt', line: 2, limit: 1 });
    update({ sessionUpdate: 'tool_call', toolCallId: 't1', title: 'a.txt bearbeiten', kind: 'edit', status: 'pending',
      content: [{ type: 'diff', path: 'C:\\x\\a.txt', oldText: read.result.content, newText: 'neu\n' }] });
    const perm = await ask('session/request_permission', { sessionId: 's1', toolCall: { toolCallId: 't1' },
      options: [{ optionId: 'ok', name: 'Erlauben', kind: 'allow_once' }, { optionId: 'no', name: 'Ablehnen', kind: 'reject_once' }] });
    if (perm.result.outcome.outcome === 'selected' && perm.result.outcome.optionId === 'ok') {
      await ask('fs/write_text_file', { sessionId: 's1', path: 'C:\\x\\a.txt', content: 'neu\n' });
      update({ sessionUpdate: 'tool_call_update', toolCallId: 't1', status: 'completed' });
    }
    send({ id: m.id, result: { stopReason: 'end_turn' } });
  }
});
"#;

fn wait_for(s: &mut AgentSession, files: &mut Vec<FileRequest>, mut done: impl FnMut(&AgentSession, &[FileRequest]) -> bool) {
    let start = Instant::now();
    loop {
        files.extend(s.poll());
        if done(s, files) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(15), "Zeitüberschreitung, Phase {:?}, Log {:?}", s.phase, s.log);
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn fake_session() -> (tempfile::TempDir, AgentSession) {
    let tmp = tempfile::tempdir().unwrap();
    let script = tmp.path().join("agent.js");
    std::fs::write(&script, FAKE_AGENT).unwrap();
    let cfg = AgentConfig { name: "Fake".into(), command: "node".into(), args: vec![script.to_string_lossy().into()], ..Default::default() };
    let s = AgentSession::start(cfg, tmp.path().to_path_buf(), || {});
    (tmp, s)
}

#[test]
fn prompt_with_tool_call_permission_and_files() {
    let (_tmp, mut s) = fake_session();
    let mut files = Vec::new();
    wait_for(&mut s, &mut files, |s, _| s.phase == Phase::Ready);
    assert_eq!(s.agent_info.as_deref(), Some("fake 1.0"));
    assert_eq!(s.modes.as_ref().map(|m| m.available_modes.len()), Some(1));

    s.prompt("Mach was", &[]);
    assert!(s.is_working());
    // The agent reads a file: lines 2..3 of the buffer.
    wait_for(&mut s, &mut files, |_, f| !f.is_empty());
    let Some(FileRequest::Read { rpc_id, line, limit, .. }) = files.pop() else { panic!("Leseanfrage erwartet") };
    let text = super::session::slice_lines("eins\nzwei\ndrei\n", line, limit);
    assert_eq!(text, "zwei\n");
    s.reply_file(&rpc_id, Ok(Some(text)));

    // Then asks before editing.
    wait_for(&mut s, &mut files, |s, _| !s.permissions.is_empty());
    let p = &s.permissions[0];
    assert_eq!(p.tool.title, "a.txt bearbeiten");
    assert_eq!(p.options.len(), 2);
    s.answer_permission(Some("ok"));

    wait_for(&mut s, &mut files, |_, f| !f.is_empty());
    let Some(FileRequest::Write { rpc_id, content, .. }) = files.pop() else { panic!("Schreibanfrage erwartet") };
    assert_eq!(content, "neu\n");
    s.reply_file(&rpc_id, Ok(None));

    wait_for(&mut s, &mut files, |s, _| s.phase == Phase::Ready);
    // Streamed text is merged into one entry; the tool call got its update.
    assert!(s.entries.iter().any(|e| matches!(e, Entry::Agent(t) if t == "Hallo Welt")));
    assert!(s.entries.iter().any(|e| matches!(e, Entry::Plan(p) if p.len() == 2 && p[0].done && p[1].active)));
    assert!(s.entries.iter().any(|e| matches!(e, Entry::Tool(t) if t.status == super::schema::ToolCallStatus::Completed && t.content.len() == 1)));
}

#[test]
fn cancel_ends_the_turn() {
    let (_tmp, mut s) = fake_session();
    let mut files = Vec::new();
    wait_for(&mut s, &mut files, |s, _| s.phase == Phase::Ready);
    s.prompt("warte", &[]);
    s.cancel();
    wait_for(&mut s, &mut files, |s, _| s.phase == Phase::Ready);
    assert!(s.entries.iter().any(|e| matches!(e, Entry::Notice { text, .. } if text == "Abgebrochen.")));
}

#[test]
fn missing_program_fails_cleanly() {
    let cfg = AgentConfig { name: "X".into(), command: "gibt-es-nicht-xyz".into(), ..Default::default() };
    let s = AgentSession::start(cfg, std::env::temp_dir(), || {});
    assert!(matches!(s.phase, Phase::Failed(_)));
}

#[test]
fn split_args_keeps_quoted_parts() {
    assert_eq!(super::split_args(r#"-y "a b" c"#), vec!["-y", "a b", "c"]);
    assert_eq!(super::split_args(r#"x """#), vec!["x", ""]);
    assert!(super::split_args("   ").is_empty());
}

//! Commit message from the diff (✦ in the Git tab): the agent of the panel
//! writes it in a separate, invisible session, so the conversation in the
//! panel stays as it is. It may read files but changes nothing.

use eframe::egui;

use super::NebulaApp;
use crate::acp::schema::PermissionOptionKind;
use crate::acp::session::{AgentSession, Entry, FileRequest, Phase};

pub(super) struct CommitMessageJob {
    session: AgentSession,
    prompt: String,
    sent: bool,
}

impl NebulaApp {
    /// Starts writing a commit message, or stops it if one is being written.
    pub(super) fn suggest_commit_message(&mut self, staged_only: bool, ctx: &egui::Context) {
        if self.commit_msg.take().is_some() {
            return;
        }
        let Some(top) = self.sidebar.git.top.clone() else { return };
        let context = match crate::ui::git::commit_context(&top, staged_only) {
            Ok(c) => c,
            Err(e) => {
                self.error(t!("Git: {e}", e = e));
                return;
            }
        };
        if context.diff.trim().is_empty() && context.new_files.is_empty() {
            self.info(t!("Keine Änderungen für einen Commit."));
            return;
        }
        let cfg = self.settings.agents.iter().find(|a| a.name == self.settings.agent).or(self.settings.agents.first()).cloned();
        let Some(cfg) = cfg else {
            self.error(t!("Kein Agent eingerichtet (Einstellungen › KI-Agenten)."));
            return;
        };
        let ctx = ctx.clone();
        let session = AgentSession::start(cfg, top, move || ctx.request_repaint());
        self.commit_msg = Some(CommitMessageJob { session, prompt: prompt(&context), sent: false });
    }

    /// Drives the job; called every frame.
    pub(super) fn poll_commit_message(&mut self) {
        let Some(job) = &mut self.commit_msg else {
            self.sidebar.git_panel.suggesting = false;
            return;
        };
        self.sidebar.git_panel.suggesting = true;
        let requests = job.session.poll();
        // Reading helps understand the change; writing is not part of this.
        for req in requests {
            match req {
                FileRequest::Read { rpc_id, path, line, limit } => {
                    let result = self.agent_read(&path).map(|t| Some(crate::acp::session::slice_lines(&t, line, limit)));
                    if let Some(job) = &mut self.commit_msg {
                        job.session.reply_file(&rpc_id, result);
                    }
                }
                FileRequest::Write { rpc_id, .. } => {
                    if let Some(job) = &mut self.commit_msg {
                        job.session.reply_file(&rpc_id, Err("Writing files is not allowed while writing a commit message.".into()));
                    }
                }
            }
        }
        let Some(job) = &mut self.commit_msg else { return };
        // Anything that needs permission (edits, commands) is declined.
        while let Some(p) = job.session.permissions.front() {
            let reject = p.options.iter().find(|o| matches!(o.kind, PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways)).map(|o| o.option_id.to_string());
            job.session.answer_permission(reject.as_deref());
        }
        match &job.session.phase {
            Phase::Ready if !job.sent => {
                let prompt = job.prompt.clone();
                job.session.prompt(&prompt, &[]);
                job.sent = true;
            }
            Phase::Ready => {
                // The answer is complete.
                let text = job.session.entries.iter().rev().find_map(|e| if let Entry::Agent(t) = e { Some(t.clone()) } else { None });
                let error = job.session.entries.iter().rev().find_map(|e| match e {
                    Entry::Notice { text, error: true } => Some(text.clone()),
                    _ => None,
                });
                self.commit_msg = None;
                match text.map(|t| clean(&t)).filter(|t| !t.is_empty()) {
                    Some(message) => self.sidebar.git_panel.message = message,
                    None => self.error(error.unwrap_or_else(|| t!("Der Agent hat keine Commit-Nachricht geliefert.").into())),
                }
            }
            Phase::Failed(e) => {
                let e = e.clone();
                self.commit_msg = None;
                self.error(t!("Commit-Nachricht: {e}", e = e));
            }
            Phase::NeedsAuth(_) | Phase::NeedsNode => {
                self.commit_msg = None;
                self.error(t!("Der Agent ist nicht bereit – im KI-Panel anmelden bzw. einrichten."));
            }
            Phase::Exited => {
                self.commit_msg = None;
                self.error(t!("Der Agent wurde beendet."));
            }
            Phase::Starting | Phase::Working => {}
        }
    }
}

fn prompt(c: &crate::ui::git::CommitContext) -> String {
    let language = crate::i18n::available().into_iter().find(|(code, _)| *code == crate::i18n::current()).map(|(_, name)| name).unwrap_or_else(|| "English".into());
    let mut p = format!(
        "Write a git commit message for the changes below. Reply with the message only: no explanation, no code fences, no quotes. \
         Start with a short summary line (at most about 72 characters); for larger changes add a blank line and a few short bullet points. \
         Match the language and style of the recent commit messages; if there are none, write in {language}. Do not use any tools and do not change files."
    );
    if !c.recent.is_empty() {
        p.push_str("\n\nRecent commit messages:\n");
        p.push_str(&c.recent.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n"));
    }
    if !c.new_files.is_empty() {
        p.push_str("\n\nNew files:\n");
        p.push_str(&c.new_files.join("\n"));
    }
    if !c.diff.trim().is_empty() {
        p.push_str(&format!("\n\nChanges:\n```diff\n{}\n```", c.diff));
    }
    p
}

/// The message without fences, quotes or a "Commit message:" lead-in.
fn clean(text: &str) -> String {
    let mut t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        // Skip a language tag after the fence.
        t = rest.split_once('\n').map_or(rest, |(_, r)| r);
        t = t.trim_end().trim_end_matches("```");
    }
    let t = t.trim();
    let t = t.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(t);
    let lower = t.to_lowercase();
    let t = if lower.starts_with("commit message:") || lower.starts_with("commit-nachricht:") {
        t.split_once(':').map_or(t, |(_, r)| r)
    } else {
        t
    };
    t.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn cleans_answers() {
        assert_eq!(clean("```\nFix login\n\n- a\n```"), "Fix login\n\n- a");
        assert_eq!(clean("```text\nFix login\n```"), "Fix login");
        assert_eq!(clean("\"Fix login\""), "Fix login");
        assert_eq!(clean("Commit message: Fix login"), "Fix login");
        assert_eq!(clean("  Lesezeichen ergänzt  "), "Lesezeichen ergänzt");
    }
}

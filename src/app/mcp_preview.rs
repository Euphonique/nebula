//! MCP tools that look at the live preview (screenshot, console): they wait
//! for the preview to open, re-layout or reload, so they are answered over
//! several frames.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use eframe::egui;
use serde_json::Value;

use super::NebulaApp;
use crate::mcp::{Answer, Call};
use crate::settings::PreviewDevice;

/// Longest a job waits in total (the MCP server gives up after 15 s).
const TIMEOUT: Duration = Duration::from_secs(12);

enum Stage {
    /// Waiting for the preview to show, then until the given time.
    Settle(Instant),
    /// The capture or script runs.
    Running,
}

enum Kind {
    Shot { slot: Arc<Mutex<Option<Result<Vec<u8>, String>>>>, width: Option<u32>, restore: Option<PreviewDevice> },
    Console { slot: Arc<Mutex<Option<String>>> },
}

pub(super) struct Job {
    reply: Sender<Result<Answer, String>>,
    kind: Kind,
    stage: Stage,
    started: Instant,
}

impl NebulaApp {
    pub(super) fn mcp_start_preview_job(&mut self, call: Call) {
        let now = Instant::now();
        let mut settle = now;
        // Open the preview for the agent if it is closed.
        if !self.preview.is_visible() && call.tool == "get_preview_screenshot" {
            if !(self.settings.right_panel && self.settings.right_tab == crate::settings::RightTab::Preview) {
                self.toggle_preview();
            }
            settle = now + Duration::from_millis(1500);
        }
        let kind = match call.tool.as_str() {
            "get_preview_screenshot" => {
                let width = call.args.get("width").and_then(Value::as_u64).map(|w| w.clamp(240, 3840) as u32);
                let mut restore = None;
                if let Some(w) = width {
                    restore = Some(self.settings.preview_device);
                    self.settings.preview_device = PreviewDevice::Custom(w);
                    settle = settle.max(now + Duration::from_millis(700));
                }
                if call.args.get("reload").and_then(Value::as_bool).unwrap_or(false) {
                    self.preview.reload();
                    settle = settle.max(now + Duration::from_millis(1500));
                }
                Kind::Shot { slot: Arc::default(), width, restore }
            }
            _ => Kind::Console { slot: Arc::default() },
        };
        self.mcp_jobs.push(Job { reply: call.reply, kind, stage: Stage::Settle(settle), started: now });
    }

    /// Moves waiting jobs on; called every frame.
    pub(super) fn mcp_preview_jobs(&mut self, ctx: &egui::Context) {
        if self.mcp_jobs.is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        let now = Instant::now();
        let jobs = std::mem::take(&mut self.mcp_jobs);
        for mut job in jobs {
            let outcome: Option<Result<Answer, String>> = if now.duration_since(job.started) > TIMEOUT {
                Some(Err("The preview did not answer in time. Is a page open in Nebula's preview?".into()))
            } else {
                match job.stage {
                    Stage::Settle(until) => {
                        let ready = match job.kind {
                            Kind::Shot { .. } => self.preview.is_visible(),
                            Kind::Console { .. } => self.preview.is_ready(),
                        };
                        if !ready && !matches!(job.kind, Kind::Shot { .. }) {
                            Some(Err("The preview is not open. Open an HTML/PHP file and the preview panel in Nebula (or take a screenshot, which opens it).".into()))
                        } else if ready && now >= until {
                            let started = match &job.kind {
                                Kind::Shot { slot, .. } => self.preview.capture_png(slot.clone()),
                                Kind::Console { slot } => self.preview.console(slot.clone()),
                            };
                            match started {
                                Ok(()) => {
                                    job.stage = Stage::Running;
                                    None
                                }
                                Err(e) => Some(Err(e)),
                            }
                        } else {
                            None
                        }
                    }
                    Stage::Running => match &job.kind {
                        Kind::Shot { slot, width, .. } => slot.lock().ok().and_then(|mut s| s.take()).map(|png| {
                            png.map(|png| Answer {
                                text: match width {
                                    Some(w) => format!("Live preview rendered at a width of {w} CSS pixels."),
                                    None => "Live preview at the width of the panel.".into(),
                                },
                                png: Some(png),
                            })
                        }),
                        Kind::Console { slot } => slot.lock().ok().and_then(|mut s| s.take()).map(|json| {
                            // The script returns a JSON string; WebView2 wraps it in quotes once more.
                            let inner = serde_json::from_str::<String>(&json).unwrap_or(json);
                            Ok(Answer::from(inner))
                        }),
                    },
                }
            };
            match outcome {
                Some(result) => {
                    if let Kind::Shot { restore: Some(device), .. } = job.kind {
                        self.settings.preview_device = device;
                    }
                    let _ = job.reply.send(result);
                }
                None => self.mcp_jobs.push(job),
            }
        }
    }
}

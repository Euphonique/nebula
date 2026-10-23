//! Keyboard macros: typing, keys and editing commands are recorded while
//! recording is on, and played back through the same code as real input.

use std::sync::Mutex;

use eframe::egui::{self, Event, Key};

/// One recorded step.
#[derive(Clone, Debug)]
pub enum Step {
    /// Input the editor handled (text, paste, keys).
    Event(Event),
    /// An editing command of the app, by its index in the app's list.
    Command(usize),
}

static RECORDING: Mutex<Option<Vec<Step>>> = Mutex::new(None);

pub fn recording() -> bool {
    RECORDING.lock().unwrap().is_some()
}

pub fn start() {
    *RECORDING.lock().unwrap() = Some(Vec::new());
}

/// Ends the recording and returns its steps.
pub fn stop() -> Option<Vec<Step>> {
    RECORDING.lock().unwrap().take()
}

pub fn record(step: Step) {
    if let Some(steps) = RECORDING.lock().unwrap().as_mut() {
        steps.push(step);
    }
}

/// Records the input events the editor acts on.
pub fn record_events(events: &[Event]) {
    let mut rec = RECORDING.lock().unwrap();
    let Some(steps) = rec.as_mut() else { return };
    for e in events {
        let keep = match e {
            Event::Text(_) | Event::Paste(_) | Event::Cut => true,
            Event::Ime(egui::ImeEvent::Commit(t)) => {
                steps.push(Step::Event(Event::Text(t.clone())));
                false
            }
            // Completion and "go to definition" depend on the moment, not the text.
            Event::Key { key: Key::F12, .. } => false,
            Event::Key { key: Key::Space, modifiers, .. } if modifiers.command => false,
            Event::Key { pressed: true, .. } => true,
            _ => false,
        };
        if keep {
            steps.push(Step::Event(e.clone()));
        }
    }
}

//! What the app knows while it runs. Sessions live only here, in memory.

use std::sync::RwLock;

#[derive(Clone, Debug)]
pub struct Session {
    pub account_label: String,
    pub administration: String,
}

#[derive(Default)]
pub struct AppState {
    sessions: RwLock<Vec<Session>>,
}

impl AppState {
    pub fn signed_in_labels(&self) -> Vec<String> {
        self.sessions
            .read()
            .map(|s| {
                s.iter()
                    .map(|x| format!("{} ({})", x.account_label, x.administration))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.read().map(|s| s.len()).unwrap_or(0)
    }
}

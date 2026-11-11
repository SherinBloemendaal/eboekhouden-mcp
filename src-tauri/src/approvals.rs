//! Changes an AI client asked for that wait for the user's word. A tool that would change an account's books
//! puts its proposal here and waits; the app shows it, and only "Goedkeuren" lets the request go to e-Boekhouden.
//! Nothing is kept on disk: a proposal lives until it is answered, times out or the app stops.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::oneshot;
use uuid::Uuid;

/// How long a proposal waits for an answer before the tool gives up.
pub const WAIT: Duration = Duration::from_secs(5 * 60);

/// One line of a proposal: what will be set and to what, in words the user knows from e-Boekhouden.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Line {
    pub label: String,
    pub value: String,
}

pub fn line(label: &str, value: impl ToString) -> Line {
    Line {
        label: label.to_owned(),
        value: value.to_string(),
    }
}

/// What a tool wants to do, as the user sees it before deciding.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: u64,
    pub account_id: Uuid,
    pub account: String,
    /// The AI client that asked ("Claude", "Cursor").
    pub client: String,
    pub tool: String,
    /// "Mutatie toevoegen: Geld uitgegeven", "Factuur F00012 verwijderen".
    pub title: String,
    pub lines: Vec<Line>,
    /// Deleting and crediting: shown as such, and asked even when the account needs no approval otherwise.
    pub danger: bool,
    pub asked_at: u64,
}

/// What a tool hands in; the registry adds the id and the time.
pub struct Ask {
    pub account_id: Uuid,
    pub account: String,
    pub client: String,
    pub tool: String,
    pub title: String,
    pub lines: Vec<Line>,
    pub danger: bool,
}

#[derive(Default)]
pub struct Approvals {
    next: AtomicU64,
    pending: Mutex<BTreeMap<u64, (Proposal, oneshot::Sender<bool>)>>,
}

impl Approvals {
    /// Puts a proposal up for the user. The receiver gets `true` for Goedkeuren and `false` for Weigeren.
    pub fn ask(&self, ask: Ask, now_ms: u64) -> (Proposal, oneshot::Receiver<bool>) {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let proposal = Proposal {
            id,
            account_id: ask.account_id,
            account: ask.account,
            client: ask.client,
            tool: ask.tool,
            title: ask.title,
            lines: ask.lines,
            danger: ask.danger,
            asked_at: now_ms,
        };
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .unwrap()
            .insert(id, (proposal.clone(), sender));
        (proposal, receiver)
    }

    /// The user's answer. `false` when the proposal is gone already (answered, timed out, or its tool call ended).
    pub fn decide(&self, id: u64, approve: bool) -> bool {
        let entry = self.pending.lock().unwrap().remove(&id);
        match entry {
            Some((_, sender)) => sender.send(approve).is_ok(),
            None => false,
        }
    }

    /// Takes a proposal down without an answer: it timed out, or the tool call that made it stopped.
    pub fn withdraw(&self, id: u64) {
        self.pending.lock().unwrap().remove(&id);
    }

    /// Refuses every proposal of an account, when the account is removed or its writing is turned off.
    pub fn refuse_account(&self, account_id: Uuid) -> usize {
        let mut pending = self.pending.lock().unwrap();
        let ids: Vec<u64> = pending
            .iter()
            .filter(|(_, (p, _))| p.account_id == account_id)
            .map(|(id, _)| *id)
            .collect();
        for id in &ids {
            if let Some((_, sender)) = pending.remove(id) {
                let _ = sender.send(false);
            }
        }
        ids.len()
    }

    /// The waiting proposals, oldest first.
    pub fn list(&self) -> Vec<Proposal> {
        self.pending
            .lock()
            .unwrap()
            .values()
            .map(|(p, _)| p.clone())
            .collect()
    }

    pub fn count(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(account_id: Uuid, title: &str) -> Ask {
        Ask {
            account_id,
            account: "Studio Noord".into(),
            client: "Claude".into(),
            tool: "mutatie_toevoegen".into(),
            title: title.into(),
            lines: vec![line("Bedrag", "2,00")],
            danger: false,
        }
    }

    #[tokio::test]
    async fn an_answer_reaches_the_waiting_tool_once() {
        let approvals = Approvals::default();
        let account = Uuid::new_v4();
        let (first, yes) = approvals.ask(ask(account, "Eerste"), 1);
        let (second, no) = approvals.ask(ask(account, "Tweede"), 2);
        assert_eq!(
            approvals.list().iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![first.id, second.id],
            "oldest first"
        );
        assert!(approvals.decide(first.id, true));
        assert!(approvals.decide(second.id, false));
        assert_eq!(yes.await, Ok(true));
        assert_eq!(no.await, Ok(false));
        assert_eq!(approvals.count(), 0);
        assert!(!approvals.decide(first.id, true), "answered already");
    }

    #[tokio::test]
    async fn a_withdrawn_proposal_can_no_longer_be_approved() {
        let approvals = Approvals::default();
        let (proposal, receiver) = approvals.ask(ask(Uuid::new_v4(), "Te laat"), 1);
        approvals.withdraw(proposal.id);
        assert!(!approvals.decide(proposal.id, true));
        assert!(
            receiver.await.is_err(),
            "the tool hears that nobody will answer"
        );
    }

    #[tokio::test]
    async fn turning_an_account_off_refuses_what_it_had_waiting() {
        let approvals = Approvals::default();
        let (gone, other) = (Uuid::new_v4(), Uuid::new_v4());
        let (_, refused) = approvals.ask(ask(gone, "Van het account"), 1);
        let (kept, _receiver) = approvals.ask(ask(other, "Van een ander"), 2);
        assert_eq!(approvals.refuse_account(gone), 1);
        assert_eq!(refused.await, Ok(false));
        assert_eq!(
            approvals.list().iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![kept.id]
        );
    }
}

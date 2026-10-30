//! Scripted terminal host for tests. Hooks run once, in the order they were queued.
use super::super::terminal::OpenOutcome;
use super::super::{json, terminal_daemon::Host, Error, Repository, Result, Value};
use super::Terminals;
use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, Mutex},
};

type SendHook = Box<dyn FnOnce(&str) -> Result<()> + Send>;
type OpenHook = Box<dyn FnOnce(&Value, &Path) -> Result<OpenOutcome> + Send>;

pub struct Fake {
    rows: Mutex<Result<Vec<Value>>>,
    reads: Mutex<usize>,
    before_read: Mutex<VecDeque<Box<dyn FnOnce() + Send>>>,
    on_send: Mutex<VecDeque<SendHook>>,
    sent: Mutex<Vec<(String, String)>>,
    on_open: Mutex<VecDeque<OpenHook>>,
    opened: Mutex<usize>,
}

impl Fake {
    /// Replaces the repository's agent terminals with a fake that has none.
    pub fn install(repo: &mut Arc<Repository>) -> Arc<Self> {
        let fake = Arc::new(Self {
            rows: Mutex::new(Ok(vec![])),
            reads: Mutex::new(0),
            before_read: Mutex::new(VecDeque::new()),
            on_send: Mutex::new(VecDeque::new()),
            sent: Mutex::new(vec![]),
            on_open: Mutex::new(VecDeque::new()),
            opened: Mutex::new(0),
        });
        Arc::get_mut(repo)
            .expect("install the fake before sharing the repository")
            .terminals = Terminals::new(Host::Fake(fake.clone()));
        fake
    }
    pub fn rows(&self, rows: Vec<Value>) {
        *self.rows.lock().unwrap() = Ok(rows);
    }
    /// Only one terminal, live and keyed by `id`.
    pub fn live(&self, id: &Value, generation: &str) {
        self.rows(vec![
            json!({"recordId":id,"generation":generation,"live":true}),
        ]);
    }
    pub fn fail(&self, message: &str) {
        *self.rows.lock().unwrap() = Err(Error::new(message));
    }
    pub fn before_read(&self, hook: impl FnOnce() + Send + 'static) {
        self.before_read.lock().unwrap().push_back(Box::new(hook));
    }
    pub fn on_send(&self, hook: impl FnOnce(&str) -> Result<()> + Send + 'static) {
        self.on_send.lock().unwrap().push_back(Box::new(hook));
    }
    /// Decides the next open. Opens without a hook are accepted.
    pub fn on_open(
        &self,
        hook: impl FnOnce(&Value, &Path) -> Result<OpenOutcome> + Send + 'static,
    ) {
        self.on_open.lock().unwrap().push_back(Box::new(hook));
    }
    pub fn opened(&self) -> usize {
        *self.opened.lock().unwrap()
    }
    pub fn reads(&self) -> usize {
        *self.reads.lock().unwrap()
    }
    pub fn sent(&self) -> Vec<(String, String)> {
        self.sent.lock().unwrap().clone()
    }
    pub(in crate::backend) fn inventory(&self) -> Result<Vec<Value>> {
        *self.reads.lock().unwrap() += 1;
        let hook = self.before_read.lock().unwrap().pop_front();
        if let Some(hook) = hook {
            hook();
        }
        self.rows.lock().unwrap().clone()
    }
    pub(in crate::backend) fn input(&self, id: &str, data: &str) -> Result<()> {
        self.sent.lock().unwrap().push((id.into(), data.into()));
        let hook = self.on_send.lock().unwrap().pop_front();
        hook.map_or(Ok(()), |hook| hook(data))
    }
    pub(in crate::backend) fn open(&self, record: &Value, packet: &Path) -> Result<OpenOutcome> {
        *self.opened.lock().unwrap() += 1;
        let hook = self.on_open.lock().unwrap().pop_front();
        hook.map_or(Ok(OpenOutcome::Accepted), |hook| hook(record, packet))
    }
}

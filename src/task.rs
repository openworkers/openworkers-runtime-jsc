use bytes::Bytes;
use std::collections::HashMap;

/// HTTP Request data
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Bytes>,
}

/// HTTP Response data
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Option<Bytes>,
}

/// Fetch event initialization data
#[derive(Debug)]
pub struct FetchInit {
    pub(crate) req: HttpRequest,
    pub(crate) res_tx: tokio::sync::oneshot::Sender<HttpResponse>,
}

impl FetchInit {
    pub fn new(req: HttpRequest, res_tx: tokio::sync::oneshot::Sender<HttpResponse>) -> Self {
        Self { req, res_tx }
    }
}

/// Scheduled event initialization data
#[derive(Debug)]
pub struct ScheduledInit {
    pub(crate) time: u64,
    pub(crate) res_tx: tokio::sync::oneshot::Sender<()>,
}

impl ScheduledInit {
    pub fn new(time: u64, res_tx: tokio::sync::oneshot::Sender<()>) -> Self {
        Self { time, res_tx }
    }
}

/// Task type discriminator
#[derive(Debug, Clone, Copy)]
pub enum TaskType {
    Fetch,
    Scheduled,
}

/// Task to be executed by a Worker
pub enum Task {
    Fetch(Option<FetchInit>),
    Scheduled(Option<ScheduledInit>),
}

impl Task {
    pub fn task_type(&self) -> TaskType {
        match self {
            Task::Fetch(_) => TaskType::Fetch,
            Task::Scheduled(_) => TaskType::Scheduled,
        }
    }

    /// Create a fetch task
    pub fn fetch(req: HttpRequest) -> (Self, tokio::sync::oneshot::Receiver<HttpResponse>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (Task::Fetch(Some(FetchInit::new(req, tx))), rx)
    }

    /// Create a scheduled task
    pub fn scheduled(time: u64) -> (Self, tokio::sync::oneshot::Receiver<()>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (Task::Scheduled(Some(ScheduledInit::new(time, tx))), rx)
    }
}

use super::*;
use std::collections::VecDeque;
use xai_acp_lib::{AcpClientMessage, AcpTxo};

type Calls = Arc<Mutex<Vec<(&'static str, String)>>>;

enum Gate<T> {
    Open,
    Held(Vec<T>),
}

#[derive(Clone, Copy)]
pub(super) enum Plan {
    Ready,
    HoldCreate,
    HoldOutput,
    HoldKill,
    UnsupportedKill,
    DropKill,
}

enum KillReplies {
    Accept,
    Held(Vec<AcpTxo<acp::KillTerminalResponse>>),
    Unsupported,
    Dropped,
}

struct Resource {
    id: String,
    output: String,
    status: Option<acp::TerminalExitStatus>,
    create: Gate<AcpTxo<acp::CreateTerminalResponse>>,
    output_replies: Gate<AcpTxo<acp::TerminalOutputResponse>>,
    kill_replies: KillReplies,
    waiters: Vec<AcpTxo<acp::WaitForTerminalExitResponse>>,
    output_requests: usize,
    wait_requests: usize,
    kill_requests: usize,
    releases: usize,
}

impl Resource {
    fn snapshot(&self) -> acp::TerminalOutputResponse {
        acp::TerminalOutputResponse::new(self.output.clone(), false).exit_status(self.status.clone())
    }

    fn exit(&mut self, status: acp::TerminalExitStatus) {
        self.status = Some(status.clone());
        for waiter in self.waiters.drain(..) {
            let _ = waiter.send(Ok(acp::WaitForTerminalExitResponse::new(status.clone())));
        }
    }

    fn accept_kill(&mut self, reply: AcpTxo<acp::KillTerminalResponse>) {
        if self.status.is_none() {
            self.exit(acp::TerminalExitStatus::new().signal(Some("SIGTERM".into())));
        }
        let _ = reply.send(Ok(acp::KillTerminalResponse::new()));
    }
}

pub(super) struct Stats {
    pub(super) output_requests: usize,
    pub(super) wait_requests: usize,
    pub(super) kill_requests: usize,
    pub(super) releases: usize,
    pub(super) held_outputs: usize,
    pub(super) held_creates: usize,
    pub(super) held_kills: usize,
    pub(super) exited: bool,
}

pub(super) struct Fixture {
    pub(super) adapter: Arc<AcpTerminalAdapter>,
    pub(super) calls: Calls,
    directory: tempfile::TempDir,
    resources: Arc<Mutex<Vec<Resource>>>,
    changed: Arc<tokio::sync::Notify>,
}

impl Fixture {
    pub(super) fn new(plans: Vec<Plan>) -> Self {
        let calls = Arc::default();
        Self::with_calls(plans, calls)
    }

    pub(super) fn with_calls(plans: Vec<Plan>, calls: Calls) -> Self {
        let resources = Arc::new(Mutex::new(Vec::<Resource>::new()));
        let changed = Arc::new(tokio::sync::Notify::new());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let owned = resources.clone();
        let observed = changed.clone();
        let recorded = calls.clone();
        tokio::spawn(async move {
            let mut plans: VecDeque<_> = plans.into();
            while let Some(message) = rx.recv().await {
                let mut resources = owned.lock().unwrap();
                match message {
                    AcpClientMessage::CreateTerminal(args) => {
                        let id = format!("resource-{}", resources.len());
                        let plan = plans.pop_front().unwrap_or(Plan::Ready);
                        let mut resource = Resource {
                            id: id.clone(), output: format!("output:{id}"), status: None,
                            create: Gate::Open, output_replies: Gate::Open, kill_replies: KillReplies::Accept,
                            waiters: Vec::new(), output_requests: 0, wait_requests: 0,
                            kill_requests: 0, releases: 0,
                        };
                        recorded.lock().unwrap().push(("create", id.clone()));
                        match plan {
                            Plan::HoldCreate => resource.create = Gate::Held(vec![args.response_tx]),
                            Plan::Ready | Plan::HoldOutput | Plan::HoldKill
                            | Plan::UnsupportedKill | Plan::DropKill => {
                                let _ = args.response_tx.send(Ok(acp::CreateTerminalResponse::new(id)));
                            }
                        }
                        match plan {
                            Plan::HoldOutput => resource.output_replies = Gate::Held(Vec::new()),
                            Plan::HoldKill => resource.kill_replies = KillReplies::Held(Vec::new()),
                            Plan::UnsupportedKill => resource.kill_replies = KillReplies::Unsupported,
                            Plan::DropKill => resource.kill_replies = KillReplies::Dropped,
                            Plan::Ready | Plan::HoldCreate => {}
                        }
                        resources.push(resource);
                    }
                    AcpClientMessage::TerminalOutput(args) => {
                        let id = args.request.terminal_id.0.to_string();
                        recorded.lock().unwrap().push(("get", id.clone()));
                        if let Some(resource) = resources.iter_mut().find(|resource| resource.id == id) {
                            resource.output_requests += 1;
                            let snapshot = resource.snapshot();
                            match &mut resource.output_replies {
                                Gate::Open => { let _ = args.response_tx.send(Ok(snapshot)); }
                                Gate::Held(replies) => replies.push(args.response_tx),
                            }
                        } else {
                            let _ = args.response_tx.send(Err(acp::Error::resource_not_found(Some(id))));
                        }
                    }
                    AcpClientMessage::WaitForTerminalExit(args) => {
                        let id = args.request.terminal_id.0.to_string();
                        recorded.lock().unwrap().push(("wait", id.clone()));
                        if let Some(resource) = resources.iter_mut().find(|resource| resource.id == id) {
                            resource.wait_requests += 1;
                            match &resource.status {
                                Some(status) => { let _ = args.response_tx.send(Ok(acp::WaitForTerminalExitResponse::new(status.clone()))); }
                                None => resource.waiters.push(args.response_tx),
                            }
                        } else {
                            let _ = args.response_tx.send(Err(acp::Error::resource_not_found(Some(id))));
                        }
                    }
                    AcpClientMessage::KillTerminalCommand(args) => {
                        let id = args.request.terminal_id.0.to_string();
                        recorded.lock().unwrap().push(("kill", id.clone()));
                        if let Some(resource) = resources.iter_mut().find(|resource| resource.id == id) {
                            resource.kill_requests += 1;
                            match &mut resource.kill_replies {
                                KillReplies::Accept => resource.accept_kill(args.response_tx),
                                KillReplies::Held(replies) => replies.push(args.response_tx),
                                KillReplies::Unsupported => { let _ = args.response_tx.send(Err(acp::Error::method_not_found())); }
                                KillReplies::Dropped => drop(args.response_tx),
                            }
                        } else {
                            let _ = args.response_tx.send(Err(acp::Error::resource_not_found(Some(id))));
                        }
                    }
                    AcpClientMessage::ReleaseTerminal(args) => {
                        let id = args.request.terminal_id.0.to_string();
                        recorded.lock().unwrap().push(("release", id.clone()));
                        if let Some(resource) = resources.iter_mut().find(|resource| resource.id == id) {
                            resource.releases += 1;
                        }
                        let _ = args.response_tx.send(Ok(acp::ReleaseTerminalResponse::new()));
                    }
                    _ => {}
                }
                observed.notify_waiters();
            }
        });
        Self {
            adapter: Arc::new(AcpTerminalAdapter::new(GatewaySender::new(tx), acp::SessionId::new("owner"))),
            calls, directory: tempfile::tempdir().unwrap(), resources, changed,
        }
    }

    pub(super) fn request(&self, owner: &str, notifications: ToolNotificationHandle) -> TerminalRunRequest {
        TerminalRunRequest {
            command: "command".into(), working_directory: self.directory.path().into(), env: HashMap::new(),
            timeout: Duration::from_secs(30), output_byte_limit: 4096,
            output_file: self.directory.path().join(format!("{}.log", uuid::Uuid::now_v7())),
            notification_handle: notifications, tool_call_id: "same-call".into(), display_command: None,
            auto_background_on_timeout: true, foreground_block_budget: Some(Duration::from_millis(10)),
            kind: TaskKind::Bash, owner_session_id: Some(owner.into()), description: Some("label".into()),
        }
    }

    pub(super) fn stats(&self, index: usize) -> Option<Stats> {
        self.resources.lock().unwrap().get(index).map(|resource| Stats {
            output_requests: resource.output_requests, wait_requests: resource.wait_requests,
            kill_requests: resource.kill_requests, releases: resource.releases,
            held_outputs: match &resource.output_replies { Gate::Open => 0, Gate::Held(replies) => replies.len() },
            held_creates: match &resource.create { Gate::Open => 0, Gate::Held(replies) => replies.len() },
            held_kills: match &resource.kill_replies {
                KillReplies::Held(replies) => replies.len(),
                KillReplies::Accept | KillReplies::Unsupported | KillReplies::Dropped => 0,
            },
            exited: resource.status.is_some(),
        })
    }

    pub(super) async fn until(&self, index: usize, predicate: impl Fn(&Stats) -> bool) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let notified = self.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.stats(index).is_some_and(|stats| predicate(&stats)) { break; }
                notified.await;
            }
        }).await.unwrap();
    }

    pub(super) fn exit(&self, index: usize, output: &str, status: acp::TerminalExitStatus) {
        let mut resources = self.resources.lock().unwrap();
        let resource = &mut resources[index];
        resource.output = output.into();
        resource.exit(status);
        self.changed.notify_waiters();
    }

    pub(super) fn open_create(&self, index: usize) {
        let mut resources = self.resources.lock().unwrap();
        let resource = &mut resources[index];
        if let Gate::Held(replies) = std::mem::replace(&mut resource.create, Gate::Open) {
            for reply in replies { let _ = reply.send(Ok(acp::CreateTerminalResponse::new(resource.id.clone()))); }
        }
        self.changed.notify_waiters();
    }

    pub(super) fn open_output(&self, index: usize) {
        let mut resources = self.resources.lock().unwrap();
        let resource = &mut resources[index];
        if let Gate::Held(replies) = std::mem::replace(&mut resource.output_replies, Gate::Open) {
            for reply in replies { let _ = reply.send(Ok(resource.snapshot())); }
        }
        self.changed.notify_waiters();
    }

    pub(super) fn open_kill(&self, index: usize) {
        let mut resources = self.resources.lock().unwrap();
        let resource = &mut resources[index];
        if let KillReplies::Held(replies) = std::mem::replace(&mut resource.kill_replies, KillReplies::Accept) {
            for reply in replies { resource.accept_kill(reply); }
        }
        self.changed.notify_waiters();
    }
}

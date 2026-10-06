//! Controllable worker boundary for coordinator tests. No packets or installed
//! kernel resources are simulated: tests supply installation completion facts
//! and inspect the actual bounded command channel used by Handle.
use super::*;

pub(in crate::runtime) struct Worker {
    pub handle: Handle,
    reports: watch::Sender<Report>,
    commands: mpsc::Receiver<Command>,
    stop: oneshot::Sender<()>,
}
impl Worker {
    pub fn new() -> Self {
        let (reports, report) = watch::channel(Report::stopped());
        let (_, reporting) = reporting::channel(reports.clone());
        let (commands, received) = mpsc::channel(4);
        let (stop, stopped) = oneshot::channel();
        let handle = Handle {
            observation_plans: watch::channel(None).0,
            diagnostic_resume_after_refresh: Default::default(),
            diagnostic_paused: watch::channel(Default::default()).0,
            commands,
            target: watch::channel(None).0,
            report,
            requests: mpsc::channel(16).1,
            worker: tokio::spawn(async {
                let _ = stopped.await;
                Ok(())
            }),
            ticket: None,
            outcome: None,
            retry: Retry::default(),
            completions: mpsc::channel(8).1,
            pending_completion: None,
            reporting,
            #[cfg(target_os = "linux")]
            traversal_acknowledged: watch::channel(Default::default()).0,
            #[cfg(target_os = "linux")]
            dns_plans: watch::channel(None).1,
            #[cfg(target_os = "linux")]
            dns_intents: Default::default(),
        };
        Self {
            handle,
            reports,
            commands: received,
            stop,
        }
    }
    pub fn install(&self, ticket: ApplicationTicket, guard: ExecutionSnapshot, ready: bool) {
        self.reports.send_modify(|report| {
            report.ticket = Some(ticket);
            report.guard = Some(guard);
            report.plane = if ready {
                DataPlane::Ready
            } else {
                DataPlane::Preparing
            };
            report.completion = ready.then_some(Ok(()));
        });
    }
    pub fn no_commands(&mut self) {
        assert!(matches!(
            self.commands.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }
    pub fn attached(&mut self, id: meshway_model::RequestId) {
        assert!(
            matches!(self.commands.try_recv(), Ok(Command::AttachPing { request, .. }) if request.id == id)
        );
    }
    pub async fn close(self) {
        self.stop.send(()).unwrap();
        self.handle.worker.await.unwrap().unwrap();
    }
}

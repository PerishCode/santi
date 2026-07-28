use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use super::Service;
use crate::turn;

#[derive(Clone)]
pub(crate) struct Control {
    stopped: watch::Sender<Option<turn::Cause>>,
}

impl Control {
    pub(crate) fn new() -> Self {
        Self {
            stopped: watch::channel(None).0,
        }
    }

    pub(crate) fn cause(&self) -> Option<turn::Cause> {
        *self.stopped.borrow()
    }

    pub(crate) fn stop(&self, cause: turn::Cause) {
        if self.cause().is_none() {
            self.stopped.send_replace(Some(cause));
        }
    }

    pub(crate) async fn wait(&self) -> turn::Cause {
        let mut stopped = self.stopped.subscribe();
        loop {
            if let Some(cause) = *stopped.borrow_and_update() {
                return cause;
            }
            if stopped.changed().await.is_err() {
                return turn::Cause::Shutdown;
            }
        }
    }
}

impl Service {
    pub async fn stop(&self, turn: &str) -> Result<Option<turn::Stop>, String> {
        let stopped = self
            .store
            .request_stop(turn, turn::Cause::Operator, &crate::now())
            .await?;
        if stopped
            .as_ref()
            .is_some_and(|stopped| stopped.turn.status == turn::Status::Running)
            && let Some(control) = self.controls.lock().unwrap().get(turn).cloned()
        {
            control.stop(turn::Cause::Operator);
        }
        Ok(stopped)
    }

    pub fn quiesce(&self, grace: Duration) {
        self.close();
        let deadline = Instant::now() + grace;
        {
            let mut held = self.deadline.lock().unwrap();
            if held.is_some_and(|current| current <= deadline) {
                return;
            }
            *held = Some(deadline);
        }
        let service = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            service.expire().await;
        });
    }

    pub(crate) fn register(&self, turn: &str) -> Control {
        let control = Control::new();
        self.controls
            .lock()
            .unwrap()
            .insert(turn.to_string(), control.clone());
        control
    }

    pub(crate) fn release(&self, turn: &str) {
        self.controls.lock().unwrap().remove(turn);
    }

    pub(crate) fn expired(&self) -> bool {
        self.deadline
            .lock()
            .unwrap()
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    pub(crate) fn halted(&self, control: &Control) -> Option<turn::Cause> {
        if self.expired() {
            control.stop(turn::Cause::Shutdown);
        }
        control.cause()
    }

    async fn expire(&self) {
        let active = self
            .controls
            .lock()
            .unwrap()
            .iter()
            .map(|(turn, control)| (turn.clone(), control.clone()))
            .collect::<Vec<_>>();
        for (turn, control) in active {
            match self
                .store
                .request_stop(&turn, turn::Cause::Shutdown, &crate::now())
                .await
            {
                Ok(Some(_)) => control.stop(turn::Cause::Shutdown),
                Ok(None) => {}
                Err(error) => {
                    self.degraded.store(true, Ordering::SeqCst);
                    eprintln!("santi: shutdown stop intent failed turn={turn} detail={error}");
                    control.stop(turn::Cause::Shutdown);
                }
            }
        }
    }

    pub async fn drain(&self) {
        loop {
            if self.controls.lock().unwrap().is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

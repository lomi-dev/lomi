use super::*;
use std::future::Future;

#[derive(Default)]
pub(super) struct Tasks {
    closed: bool,
    asynchronous: Vec<tokio::task::JoinHandle<()>>,
    blocking: Vec<tokio::task::JoinHandle<()>>,
    native: Vec<tokio::task::JoinHandle<()>>,
    native_paused: bool,
}

#[derive(Default)]
pub(super) enum Shutdown {
    #[default]
    Idle,
    Running(tokio::task::JoinHandle<()>),
    Complete,
}

impl Broker {
    /// Keep native command work owned if its renderer drops the response future.
    pub async fn native_worker<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&Broker) -> T + Send + 'static,
    ) -> Result<T, ErrorCode> {
        let (sender, receiver) = oneshot::channel();
        {
            let mut tasks = self.workers.lock().unwrap_or_else(|e| e.into_inner());
            if tasks.closed || tasks.native_paused {
                return Err(ErrorCode::ControlRevoked);
            }
            tasks.native.retain(|task| !task.is_finished());
            let broker = self.clone();
            tasks.native.push(tokio::task::spawn_blocking(move || {
                let _ = sender.send(work(&broker));
            }));
        }
        receiver.await.map_err(|_| ErrorCode::OutcomeUnknown)
    }
    /// The app revokes first, then waits for native process owners before exit.
    /// Keep their handles registered if the caller cancels its close attempt.
    pub async fn pause_native_workers(&self) {
        self.workers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .native_paused = true;
        loop {
            if self
                .workers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .native
                .iter()
                .all(|task| task.is_finished())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    pub fn resume_native_workers(&self) {
        self.workers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .native_paused = false;
    }
    pub(super) fn spawn_background(&self, future: impl Future<Output = ()> + Send + 'static) {
        let mut tasks = self.workers.lock().unwrap_or_else(|e| e.into_inner());
        if tasks.closed {
            return;
        }
        tasks.asynchronous.retain(|task| !task.is_finished());
        tasks.asynchronous.push(tokio::spawn(future));
    }

    pub(super) fn spawn_worker<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> oneshot::Receiver<T> {
        let (sender, receiver) = oneshot::channel();
        let mut tasks = self.workers.lock().unwrap_or_else(|e| e.into_inner());
        if !tasks.closed {
            tasks.blocking.retain(|task| !task.is_finished());
            tasks.blocking.push(tokio::task::spawn_blocking(move || {
                let _ = sender.send(work());
            }));
        }
        receiver
    }

    pub(super) async fn stop_workers(&self) {
        // Register under the same short lock used to close admission. Dropping a
        // connection must not detach work that still owns the receipt database.
        let (asynchronous, blocking, native) = {
            let mut tasks = self.workers.lock().unwrap_or_else(|e| e.into_inner());
            tasks.closed = true;
            (
                std::mem::take(&mut tasks.asynchronous),
                std::mem::take(&mut tasks.blocking),
                std::mem::take(&mut tasks.native),
            )
        };
        for task in &asynchronous {
            task.abort();
        }
        for task in asynchronous {
            let _ = task.await;
        }
        // Started blocking jobs cannot be aborted. Revoke has invalidated their
        // native permits; join their actual completion, including captured Arcs.
        for task in blocking.into_iter().chain(native) {
            let _ = task.await;
        }
    }
}

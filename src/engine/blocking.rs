//! Cancellable filesystem work on Tokio's blocking pool.
use std::io;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub(crate) struct Cancellation {
    stopped: Arc<AtomicBool>,
    owner: Arc<super::resources::Resources>,
}

impl Cancellation {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.stopped.load(Ordering::Acquire) || self.owner.is_closed()
    }

    pub(crate) fn check(&self) -> io::Result<()> {
        if self.is_cancelled() {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "tool scan cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

/// Dropping the caller cancels queued work and asks an active worker to stop.
/// Workers must check the token between filesystem operations; an OS call
/// already in progress cannot be preempted.
pub(crate) async fn run_cancellable<F, R>(work: F) -> io::Result<R>
where
    F: FnOnce(Cancellation) -> io::Result<R> + Send + 'static,
    R: Send + 'static,
{
    let cancel = Cancellation {
        stopped: Arc::new(AtomicBool::new(false)),
        owner: super::resources::current(),
    };
    cancel.check()?;
    struct StopOnDrop(Arc<AtomicBool>);
    impl Drop for StopOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let _stop = StopOnDrop(cancel.stopped.clone());
    let worker = super::resources::spawn_blocking(move || {
        cancel.check()?;
        let result = work(cancel.clone());
        cancel.check()?;
        result
    });
    let _abort: crate::agent::runner::RunGuard = worker.abort_handle().into();
    worker.await.map_err(|error| {
        if error.is_cancelled() {
            io::Error::new(io::ErrorKind::Interrupted, "tool scan cancelled")
        } else {
            io::Error::other(error)
        }
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test(flavor = "current_thread")]
    async fn worker_yields_the_executor_and_preserves_workspace() {
        let workspace = std::env::temp_dir().join("zs-worker-scope");
        let owner = crate::engine::resources::Resources::for_workspace(workspace.clone());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let executor_thread = std::thread::current().id();
        let (result, _) = tokio::join!(
            owner.clone().scope(run_cancellable(move |_| {
                assert_ne!(std::thread::current().id(), executor_thread);
                let _ = started_tx.send(());
                release_rx
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(std::io::Error::other)?;
                crate::engine::resources::working_dir()
            })),
            async {
                started_rx.await.unwrap();
                release_tx.send(()).unwrap();
            }
        );
        assert_eq!(result.unwrap(), workspace);
        owner.shutdown().await;
    }

    #[tokio::test]
    async fn dropping_the_caller_stops_a_running_worker() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel();
        let mut work = Box::pin(run_cancellable(move |cancel| {
            let _ = started_tx.send(());
            loop {
                if let Err(error) = cancel.check() {
                    let _ = stopped_tx.send(());
                    return Err::<(), _>(error);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }));
        tokio::select! {
            result = &mut work => panic!("worker should wait: {result:?}"),
            _ = started_rx => {},
        }
        drop(work);
        tokio::time::timeout(Duration::from_secs(2), stopped_rx)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn shutdown_stops_workers_and_rejects_new_scans() {
        let owner = crate::engine::resources::Resources::for_workspace(std::env::temp_dir());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let mut work = Box::pin(owner.clone().scope(run_cancellable(move |cancel| {
            let _ = started_tx.send(());
            while !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            cancel.check()
        })));
        tokio::select! {
            result = &mut work => panic!("worker should wait: {result:?}"),
            _ = started_rx => {},
        }
        tokio::time::timeout(Duration::from_secs(2), owner.shutdown())
            .await
            .unwrap();
        assert_eq!(
            work.await.unwrap_err().kind(),
            std::io::ErrorKind::Interrupted
        );
        let result: std::io::Result<()> = owner
            .scope(run_cancellable(|_| panic!("closed context ran scan")))
            .await;
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
    }
}

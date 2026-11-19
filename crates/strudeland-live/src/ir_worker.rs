//! The IR worker: builds reverb impulse responses off the engine thread.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use strudeland_engine::live::{IrBuilt, IrRequest, build_ir};

/// The worker thread; it ends when the request sender is dropped (or the result receiver is).
pub(crate) struct IrWorker {
    thread: Option<JoinHandle<()>>,
}

impl IrWorker {
    /// Spawns the worker. The engine gets the request sender and the result receiver
    /// (`LiveEngine::attach_ir_worker`).
    pub(crate) fn spawn() -> std::io::Result<(IrWorker, Sender<IrRequest>, Receiver<IrBuilt>)> {
        let (req_tx, req_rx) = channel::<IrRequest>();
        let (built_tx, built_rx) = channel::<IrBuilt>();
        let thread = std::thread::Builder::new()
            .name("strudeland-live-ir".into())
            .spawn(move || run(req_rx, built_tx))?;
        Ok((
            IrWorker {
                thread: Some(thread),
            },
            req_tx,
            built_rx,
        ))
    }

    /// Waits for the worker to end (call after dropping the request sender).
    pub(crate) fn join(mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(requests: Receiver<IrRequest>, built: Sender<IrBuilt>) {
    while let Ok(req) = requests.recv() {
        if built.send(build_ir(req)).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_when_the_request_sender_is_dropped() {
        let (worker, req_tx, _built_rx) = IrWorker::spawn().unwrap();
        drop(req_tx);
        worker.join();
    }
}

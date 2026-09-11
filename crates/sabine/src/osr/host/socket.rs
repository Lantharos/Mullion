use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use winit::event_loop::EventLoopProxy;

use crate::osr::message_queue::MessageQueue;
use crate::osr::protocol::read_message;
use crate::osr::transport::{IpcEndpoint, IpcListener, IpcStream};

use super::types::OsrHostEvent;

pub(super) struct SocketReader {
    state: Arc<ReaderState>,
    endpoint: IpcEndpoint,
}

struct ReaderState {
    stopped: AtomicBool,
    stream: Mutex<Option<IpcStream>>,
    messages: Arc<MessageQueue>,
}

impl Drop for SocketReader {
    fn drop(&mut self) {
        self.state.stopped.store(true, Ordering::Release);
        self.state.messages.close();
        if let Ok(stream) = self.state.stream.lock()
            && let Some(stream) = stream.as_ref()
        {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        self.endpoint.unlink();
    }
}

impl ReaderState {
    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    fn send(&self, sender: &mpsc::SyncSender<OsrHostEvent>, mut event: OsrHostEvent) -> bool {
        while !self.stopped() {
            match sender.try_send(event) {
                Ok(()) => return true,
                Err(mpsc::TrySendError::Disconnected(_)) => return false,
                Err(mpsc::TrySendError::Full(pending)) => {
                    event = pending;
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
        false
    }
}

pub(super) fn start_socket_reader(
    generation: u64,
    listener: IpcListener,
    endpoint: IpcEndpoint,
    authentication_token: String,
    sender: mpsc::SyncSender<OsrHostEvent>,
    proxy: EventLoopProxy,
) -> std::io::Result<SocketReader> {
    if let Err(error) = listener.set_nonblocking(true) {
        endpoint.unlink();
        return Err(error);
    }
    let state = Arc::new(ReaderState {
        stopped: AtomicBool::new(false),
        stream: Mutex::new(None),
        messages: Arc::new(MessageQueue::new()),
    });
    let reader = SocketReader {
        state: Arc::clone(&state),
        endpoint: endpoint.clone(),
    };
    thread::spawn(move || {
        let messages = Arc::clone(&state.messages);
        let mut stream = loop {
            if state.stopped() {
                return;
            }
            let mut candidate = match listener.accept() {
                Ok((candidate, _)) => candidate,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
                Err(_) => {
                    endpoint.unlink();
                    state.send(&sender, OsrHostEvent::Disconnected(generation));
                    proxy.wake_up();
                    return;
                }
            };
            if let Err(error) = candidate.set_read_timeout(Some(Duration::from_millis(750))) {
                eprintln!("Sabine OSR could not set authentication deadline: {error}");
                continue;
            }
            match crate::osr::transport::authenticate(&mut candidate, &authentication_token) {
                Ok(crate::osr::transport::Authentication::Accepted) => {
                    if let Err(error) = candidate.set_read_timeout(None) {
                        eprintln!("Sabine OSR could not clear authentication deadline: {error}");
                        continue;
                    }
                    break candidate;
                }
                Ok(crate::osr::transport::Authentication::Probe) => continue,
                Err(error) => {
                    eprintln!("Sabine OSR reject connect: {error}");
                }
            }
        };
        let Ok(writer) = stream.try_clone() else {
            endpoint.unlink();
            state.send(&sender, OsrHostEvent::Disconnected(generation));
            proxy.wake_up();
            return;
        };
        let owned = match stream.try_clone() {
            Ok(owned) => owned,
            Err(error) => {
                eprintln!("Sabine OSR could not own reader socket: {error}");
                let _ = stream.shutdown(std::net::Shutdown::Both);
                endpoint.unlink();
                state.send(&sender, OsrHostEvent::Disconnected(generation));
                proxy.wake_up();
                return;
            }
        };
        if let Ok(mut current) = state.stream.lock() {
            *current = Some(owned);
        }
        if !state.send(&sender, OsrHostEvent::Connected(generation, writer)) {
            return;
        }
        proxy.wake_up();
        while !state.stopped() {
            match read_message(&mut stream) {
                Ok(Some(message)) => {
                    if messages.push(message) {
                        if !state.send(
                            &sender,
                            OsrHostEvent::MessagesReady(generation, Arc::clone(&messages)),
                        ) {
                            break;
                        }
                        proxy.wake_up();
                    }
                }
                Ok(None) => break,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                    ) =>
                {
                    break;
                }
                Err(error) => {
                    eprintln!("Sabine OSR socket read failed: {error}");
                    break;
                }
            }
        }
        endpoint.unlink();
        state.send(&sender, OsrHostEvent::Disconnected(generation));
        proxy.wake_up();
    });
    Ok(reader)
}

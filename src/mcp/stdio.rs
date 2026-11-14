use std::process::Stdio;
use std::time::Duration;

use fnv::FnvHashMap;
use futures::future::BoxFuture;
use log::{debug, error};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::oneshot::{Receiver as OneshotReceiver, Sender as OneshotSender, channel};
use tokio::sync::mpsc::{UnboundedReceiver as Receiver, UnboundedSender as Sender, unbounded_channel};
use tokio::time::Instant;

use crate::json_rpc::{Request, RequestId, Response, ResponseBody};
use crate::mcp::transport::{Transport, TransportError};

#[derive(Debug)]
enum ReceiveError {
    Terminated,
    Temporary(std::io::Error),
}

impl From<std::io::Error> for ReceiveError {
    fn from(value: std::io::Error) -> Self {
        Self::Temporary(value)
    }
}

type ResponseMessage = ResponseBody;

/// Maximum number of times we will retry a pipe failure (dead process)
/// before we treat the failure as unrecoverable.
const MAX_PIPE_FAILS: usize = 5;

#[derive(Debug)]
enum Message {
    Opened {
        request: Request,
        response_channel: OneshotSender<ResponseMessage>,
    },
    // We rely on clients reporting their own disconnections to avoid timer
    // bookkeeping in the event loop.
    Closed {
        id: RequestId,
    },
}

#[derive(Debug)]
struct Process {
    inner: Child,
    stdout: BufReader<ChildStdout>,
    stdin: BufWriter<ChildStdin>,
}

fn build_command(args: &[String]) -> Command {
    let mut command = Command::new(&args[0]);
    command.args(&args[1..])
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    command
}

/// Manages MCP stdio process lifetime and serves both requests and responses.
#[derive(Debug)]
struct StdioEventLoop {
    /// Name used in error messages
    name: String,
    command: Command,
    process: Option<Process>,
    request_channel: Receiver<Message>,
    response_channels: FnvHashMap<RequestId, OneshotSender<ResponseMessage>>,
}

impl Drop for StdioEventLoop {
    fn drop(&mut self) {
        todo!()
    }
}

impl StdioEventLoop {
    fn new(name: String, command: Command) -> (Self, Sender<Message>) {
        let (sender, request_channel) = unbounded_channel::<Message>();
        let ev = Self {
            name,
            command,
            process: None,
            request_channel,
            response_channels: Default::default(),
        };
        (ev, sender)
    }

    async fn spawn(&mut self) -> std::io::Result<()> {
        if self.process.is_none() {
            let mut child = self.command.spawn()?;
            self.process = Some(Process {
                stdin: BufWriter::new(child.stdin.take().unwrap()),
                stdout: BufReader::new(child.stdout.take().unwrap()),
                inner: child,
            });
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.process = None;
        // Disconnects all channels, which will cause clients to retry
        self.response_channels.clear();
    }

    async fn run(mut self) {
        loop {
            for _ in 0..MAX_PIPE_FAILS {
                match self.attempt_select().await {
                    Ok(_) => break, // Reset pipe fail counter
                    Err(ReceiveError::Temporary(e)) => {
                        error!("mcp server '{}': {}", self.name, e);
                    },
                    Err(ReceiveError::Terminated) => {
                        debug!("mcp server '{}': terminated", self.name);
                        return;
                    }
                }
                self.reset();
            }
            error!("mcp server '{}': aborting after {} consecutive failures", self.name, MAX_PIPE_FAILS);
            return;
        }
    }

    // Returns `Ok(())` after successfully handling a request or response.
    async fn attempt_select(&mut self) -> Result<(), ReceiveError> {
        self.spawn().await?;
        let mut line = String::new();
        loop {
            tokio::select! {
                res = self.process.as_mut().unwrap().stdout.read_line(&mut line) => {
                    res?;
                    self.handle_response(&line).await?;
                    line.clear();
                    return Ok(());
                },
                message = self.request_channel.recv() => {
                    // If None, channel is permanently closed
                    let message = message.ok_or(ReceiveError::Terminated)?;
                    let is_request = matches!(message, Message::Opened { .. });
                    self.handle_message(message).await?;
                    if is_request {
                        return Ok(());
                    } else {
                        continue;
                    }
                },
            }
        }
    }

    async fn handle_message(&mut self, message: Message) -> std::io::Result<()> {
        match message {
            Message::Opened { request, response_channel } => {
                let Some(id) = request.id.clone() else { return Ok(()) };
                self.response_channels.insert(id, response_channel);
                self.forward_request(&request).await?;
            },
            Message::Closed { id } => {
                self.response_channels.remove(&id);
            }
        }
        Ok(())
    }

    async fn forward_request(&mut self, request: &Request) -> std::io::Result<()> {
        let mut bytes = serde_json::to_vec(&request).unwrap();
        bytes.push('\n' as u8);
        self.process.as_mut().unwrap().stdin.write_all(&bytes).await?;
        Ok(())
    }

    async fn handle_response(&mut self, line: &str) -> std::io::Result<()> {
        let response: Response = serde_json::from_str(line)?;
        if let Some(channel) = self.response_channels.remove(&response.id) {
            let _ = channel.send(response.body);
        }
        Ok(())
    }
}

/// Resubmits a request to the event loop until timeout is reached or the event
/// loop dies.
#[derive(Debug)]
struct StdioConnection {
    expire_at: Instant,
    request: Request,
    send: Option<Sender<Message>>,
}

impl Drop for StdioConnection {
    fn drop(&mut self) {
        // We only send a connection close message when dropping if the channel
        // is still open and we haven't received a response.
        if let Some(send) = self.send.take()
            && let Some(id) = self.request.id.take()
        {
            let _ = send.send(Message::Closed { id });
        }
    }
}

impl StdioConnection {
    fn new(
        send: Sender<Message>,
        request: Request,
        timeout: Duration,
    ) -> Self {
        Self {
            expire_at: Instant::now() + timeout,
            request,
            send: Some(send),
        }
    }

    fn open(&mut self) -> Result<OneshotReceiver<ResponseMessage>, TransportError> {
        let (response_send, response_recv) = channel();
        let message = Message::Opened {
            request: self.request.clone(),
            response_channel: response_send,
        };
        let res = self.send.as_mut().unwrap().send(message);
        match res {
            Ok(()) => Ok(response_recv),
            // Other end of pipe is closed, no more requests can be sent
            Err(_) => Err(TransportError::Unrecoverable),
        }
    }

    async fn recv(mut self) -> Result<ResponseBody, TransportError> {
        while Instant::now() < self.expire_at {
            let mut recv = self.open()?;

            let timeout = self.expire_at - Instant::now();
            let res = tokio::time::timeout(timeout, &mut recv).await;
            match res {
                Ok(Ok(response)) => {
                    self.send.take(); // Prevent close message from being sent
                    return Ok(response);
                }
                Ok(Err(_recv_error)) => continue, // Server restarted
                Err(_timeout) => return Err(TransportError::Timeout),
            }
        }
        Err(TransportError::Timeout)
    }
}

#[derive(Debug)]
pub(crate) struct StdioTransport {
    sender: Sender<Message>,
}

impl StdioTransport {
    pub(crate) fn new(
        name: String,
        command: &[String],
    ) -> Self {
        let command = build_command(&command);
        let (event_loop, sender) = StdioEventLoop::new(name, command);
        tokio::task::spawn(event_loop.run());
        Self {
            sender,
        }
    }
}

impl Transport for StdioTransport {
    fn request(
        &self,
        request: Request,
        timeout: Duration,
    ) -> BoxFuture<'static, Result<ResponseBody, TransportError>> {
        Box::pin(StdioConnection::new(
            self.sender.clone(),
            request,
            timeout,
        ).recv())
    }
}

use std::io;
use std::time::Duration;

use crossterm::event::Event;

pub type AppResult<T> = io::Result<T>;

/// Events delivered to the UI loop.
#[derive(Debug)]
pub enum AppEvent {
    Term(Event),
    /// A message arriving from the (simulated) network.
    Message(String),
    Quit,
}

/// The mutable application state.
pub struct App {
    pub messages: Vec<String>,
    pub input: String,
    /// Inbound channel for events from background tasks.
    pub events: smol::channel::Receiver<AppEvent>,
    /// Sender kept so spawned tasks can post events back to the UI.
    event_tx: smol::channel::Sender<AppEvent>,
    /// Outbound channel to the network task (user-submitted messages).
    network_tx: Option<smol::channel::Sender<String>>,
}

impl App {
    pub fn new() -> Self {
        let (event_tx, events) = smol::channel::unbounded::<AppEvent>();
        let tx = event_tx.clone();
        smol::spawn(async move {
            let _ = tx
                .send(AppEvent::Message(
                    "Welcome to codequick! Type a message and press Enter. Ctrl+C or Esc to quit."
                        .into(),
                ))
                .await;
        })
        .detach();
        Self {
            messages: Vec::new(),
            input: String::new(),
            events,
            event_tx,
            network_tx: None,
        }
    }

    /// Spawn a dummy "network" task that echoes messages back after a delay.
    /// Returns the sender so the UI can forward user input.
    pub fn spawn_network_task(&mut self) -> smol::channel::Sender<AppEvent> {
        let (net_tx, net_rx) = smol::channel::unbounded::<String>();
        let event_tx = self.event_tx.clone();

        smol::spawn(async move {
            while let Ok(msg) = net_rx.recv().await {
                // Simulate network round-trip latency.
                smol::Timer::after(Duration::from_millis(400)).await;
                let _ = event_tx.send(AppEvent::Message(format!("[echo] {msg}"))).await;
            }
        })
        .detach();

        self.network_tx = Some(net_tx);

        // The UI loop already has the events receiver; we return a clone of
        // the event sender so it can post a Quit when shutting down.
        self.event_tx.clone()
    }

    pub fn backspace(&mut self) {
        self.input.pop();
    }

    pub fn submit_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.messages.push(format!("you: {text}"));
        self.input.clear();
        if let Some(tx) = &self.network_tx {
            let _ = tx.send_blocking(text);
        }
    }

    pub fn handle_event(&mut self, ev: AppEvent) {
        if let AppEvent::Message(msg) = ev {
            self.messages.push(msg);
        }
    }
}

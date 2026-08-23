use std::time::Duration;

use diesel::SqliteConnection;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::app::AppEvent;
use crate::request::DefaultClient;
use crate::session::{Content, Item};

pub struct Agent {
    pub item: Item,
    pub content: Content,
    pub sender: UnboundedSender<AppEvent>,
    pub client: DefaultClient,
    pub conn: SqliteConnection,
    pub cancel: CancellationToken,
}

impl Agent {
    pub async fn run(self) {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            tokio::select! {
                _ = self.cancel.cancelled() => return,
                _ = sleep(Duration::from_millis(25)) => {}
            }
        }
        let _ = self.sender.send(AppEvent::ContentCreated {
            item: self.item,
            content: self.content,
        });
    }

    pub fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(async { self.run().await })
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::unbounded_channel;

    use crate::session::{ContentType, ItemType, Session};

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn test_spawn_cancels() {
        let mut conn = crate::db::open_new().unwrap();
        let session = Session::create(&mut conn, "Session").expect("create session");
        let item = Item::create(&mut conn, session.id, None, ItemType::User, None)
            .expect("create item");
        let content = Content::create(&mut conn, item.id, ContentType::Text, "hello")
            .expect("create content");

        let cancel = CancellationToken::new();
        let (sender, mut recv) = unbounded_channel();
        let agent = Agent {
            item,
            content,
            sender,
            client: DefaultClient::default(),
            conn,
            cancel: cancel.clone(),
        };
        let task = agent.spawn();
        cancel.cancel();
        task.await.unwrap();
        assert!(recv.try_recv().is_err());
    }
}

use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::app::AppEvent;
use crate::session::{Content, Item};

pub fn spawn(
    item: Item,
    content: Content,
    sender: UnboundedSender<AppEvent>,
) -> (JoinHandle<()>, CancellationToken) {
    let cancel = CancellationToken::new();
    let handle = tokio::spawn(dummy_task(cancel.clone(), sender, item, content));
    (handle, cancel)
}

async fn dummy_task(
    cancel: CancellationToken,
    events: UnboundedSender<AppEvent>,
    item: Item,
    content: Content,
) {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = sleep(Duration::from_millis(25)) => {}
        }
    }
    let _ = events.send(AppEvent::ContentCreated { item, content });
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

        let (send, mut recv) = unbounded_channel();
        let (task, cancel) = spawn(item, content, send);
        cancel.cancel();
        task.await.unwrap();
        assert!(recv.try_recv().is_err());
    }
}

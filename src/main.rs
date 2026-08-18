use nagai::app;
use nagai::db;
use nagai::error::AnyResult;
use nagai::model::revalidate_models;

#[tokio::main]
async fn main() -> AnyResult<()> {
    tokio::spawn(async {
        let mut conn = match db::open() {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("failed to open db for revalidation: {e}");
                return;
            }
        };
        if let Err(e) = revalidate_models(&mut conn).await {
            eprintln!("failed to revalidate models: {e}");
        }
    });

    app::run().await
}

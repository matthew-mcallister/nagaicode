use nagai::app;
use nagai::error::AnyResult;

#[tokio::main]
async fn main() -> AnyResult<()> {
    app::run().await
}

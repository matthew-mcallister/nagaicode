use nagai::app;
use nagai::error::AnyResult;
use nagai::logging::init_logging;

#[tokio::main]
async fn main() -> AnyResult<()> {
    init_logging()?;
    app::run().await
}

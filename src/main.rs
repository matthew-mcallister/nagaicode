#![feature(iter_macro, yield_expr)]

use nagai::error::AnyResult;
use nagai::ui;

fn main() -> AnyResult<()> {
    ui::run()
}

use crossterm::event::{Event, KeyCode, KeyEvent};
use nagai::app::App;
use nagai::terminal::TestTerminal;
use nagai::ui::canvas::render_canvas;

const EXPECTED_INITIAL_FRAME: &str = concat!(
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[38;2;168;162;158m  ▐ \x1b[3mWelcome to NagaiCode!                                                  \x1b[38;2;87;83;78m\x1b[23m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[38;2;168;162;158m  ▐ \x1b[3m                                                                       \x1b[38;2;87;83;78m\x1b[23m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[38;2;168;162;158m  ▐ \x1b[3mType /help for a list of commands.                                     \x1b[38;2;87;83;78m\x1b[23m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                             \x1b[38;2;87;83;78m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[48;2;41;37;36m                                                                           \x1b[38;2;56;189;248m\x1b[48;2;12;10;9m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[48;2;41;37;36m                                                                           \x1b[38;2;56;189;248m\x1b[48;2;12;10;9m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[48;2;41;37;36m                                                                           \x1b[38;2;56;189;248m\x1b[48;2;12;10;9m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                ",
);

#[test]
fn test_app_e2e() {
    let terminal = TestTerminal::new();
    let mut app = App::new(terminal).expect("failed to create app");

    let mut canvas = app.make_canvas();
    app.draw(&mut canvas);
    let frame = render_canvas(&mut canvas);

    assert_eq!(frame, EXPECTED_INITIAL_FRAME);
    assert!(!app.quit());

    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char('/'))));
    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char('q'))));
    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Enter)));

    assert!(app.quit());
}

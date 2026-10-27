use montty::app::App;
use montty::cli;
use montty::term::pane::UserEvent;
use winit::event_loop::EventLoop;

fn main() {
    // Hidden: the image decoder helper (see montty::decoder). No event loop, no display.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("--decode-image") {
        std::process::exit(montty::decoder::run_helper(&argv[1..]));
    }
    let args = match cli::parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("montty: {e}");
            eprintln!("{}", cli::USAGE);
            std::process::exit(2);
        }
    };
    if args.help {
        println!("{}", cli::USAGE);
        return;
    }
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("montty: cannot open display: {e}");
            std::process::exit(1);
        }
    };
    montty::device_events::ignore(&event_loop);
    let mut app = App::new(args, event_loop.create_proxy());
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("montty: {e}");
        std::process::exit(1);
    }
    if let Some(e) = app.error {
        eprintln!("montty: {e}");
        std::process::exit(1);
    }
}

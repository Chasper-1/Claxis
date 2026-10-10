use claxis_term::{Event, KeyCode, Modifiers, Terminal};

fn main() {
    let mut term = match Terminal::new() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    loop {
        match term.next_event() {
            Ok(Event::KeyPressed { key, .. })
                if key.code == KeyCode::Char('q') && key.modifiers == Modifiers::CTRL =>
            {
                break;
            }
            Ok(event) => println!("{event:?}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
}

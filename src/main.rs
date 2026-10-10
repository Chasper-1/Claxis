use claxis::api::term::{Event, KeyCode, Modifiers, Terminal};
use claxis::main_messages::{En, Messages};
use claxis::router::Editor;

fn main() {
    let messages = En;

    // Терминал: без протокола событий отпускания модель ввода не работает.
    let mut term = match Terminal::new() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}", messages.terminal_failed(&e.to_string()));
            std::process::exit(1);
        }
    };

    // Редактор: конфиг с диска, настройки из него, сессия под них.
    let editor = match Editor::open() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("{}", messages.session_failed(&e.to_string()));
            std::process::exit(1);
        }
    };

    // Сломанный конфиг не молчит: сообщаем, что пошли на запасной.
    if editor.used_last_good {
        eprintln!(
            "{}",
            messages.event_line("config is broken, using the last saved good copy")
        );
    }
    for issue in &editor.issues {
        eprintln!("{}", messages.event_line(&issue.text()));
    }

    loop {
        match term.next_event() {
            Ok(Event::KeyPressed { key, .. })
                if key.code == KeyCode::Char('q') && key.modifiers == Modifiers::CTRL =>
            {
                break;
            }
            Ok(event) => println!("{}", messages.event_line(&format!("{event:?}"))),
            Err(e) => {
                eprintln!("{}", messages.terminal_failed(&e.to_string()));
                std::process::exit(1);
            }
        }
    }
}

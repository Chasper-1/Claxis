use claxis::api::term::{Event, KeyCode, Modifiers, Terminal};
use claxis::main_messages::{En, Messages};
use claxis::router::Session;

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

    // Сессия: хранилище снапшотов и настройки из конфигов крейтов.
    let session = match Session::open_default() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", messages.session_failed(&e.text()));
            std::process::exit(1);
        }
    };
    println!("{}", messages.event_line(&format!("{session:?}")));

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

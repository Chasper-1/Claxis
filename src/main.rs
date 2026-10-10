use std::io::Write;

use claxis::api::term::{Event, KeyCode, Modifiers, Terminal};
use claxis::cli::{self, Command};
use claxis::main_messages::{En, Messages};
use claxis::router::Editor;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Команды разбираются до терминала: `claxis messages` должен работать
    // в обычной оболочке, где kitty-протокола нет.
    match cli::parse(&args) {
        Command::Messages => {
            let mut out = std::io::stdout().lock();
            if let Err(e) = cli::print_messages(&mut out).and_then(|()| out.flush()) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Command::Help => {
            let mut out = std::io::stdout().lock();
            let _ = cli::print_help(&mut out).and_then(|()| out.flush());
        }
        // Редактор открывается с терминалом, дальше идёт обычный цикл.
        Command::Run => run(),
    }
}

fn run() {
    let messages = En;

    let mut term = match Terminal::new() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}", messages.terminal_failed(&e.to_string()));
            std::process::exit(1);
        }
    };

    let mut editor = match Editor::open() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("{}", messages.session_failed(&e.to_string()));
            std::process::exit(1);
        }
    };

    // О сломанном конфиге говорим сразу: молчать нельзя, иначе пользователь
    // не увидит, что его правка не сработала.
    if editor.used_last_good {
        eprintln!(
            "{}",
            messages.event_line("config is broken, using the last saved good copy")
        );
    }
    for issue in editor.issues.clone() {
        eprintln!("{}", messages.event_line(&issue.text()));
    }

    loop {
        // Изменение конфига приходит от системы. Пока цикл стоит на вводе,
        // новости ждут: разбудить его от файла — отдельная задача.
        if let Some(reloaded) = editor.poll_watch() {
            for issue in &reloaded.issues {
                eprintln!("{}", messages.event_line(&issue.text()));
            }
        }

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

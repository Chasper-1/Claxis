use std::io::Write;
use std::path::PathBuf;

use claxis::api::term::Terminal;
use claxis::cli::{self, Command};
use claxis::main_messages::{En, Messages};
use claxis::router::{Cycle, Editor, SessionError};

fn main() {
    let messages = En;
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Команды разбираются до терминала: `claxis messages` должен работать
    // в обычной оболочке, где kitty-протокола нет.
    let path = match cli::parse(&args) {
        Command::Messages => {
            let mut out = std::io::stdout().lock();
            if let Err(e) = cli::print_messages(&mut out).and_then(|()| out.flush()) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        Command::Help => {
            let mut out = std::io::stdout().lock();
            let _ = cli::print_help(&mut out, &messages).and_then(|()| out.flush());
            return;
        }
        // Редактор открывается с терминалом, дальше идёт обычный цикл.
        Command::Run { path } => path,
    };

    run(path);
}

fn run(path: Option<PathBuf>) {
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

    // Файла может не быть: редактор с этим умеет, и пустой документ —
    // такой же документ, как остальные.
    let mut doc = match open_document(&editor, path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{}", messages.session_failed(&e.to_string()));
            std::process::exit(1);
        }
    };

    let mut cycle = Cycle::new();
    if let Err(e) = cycle.run(&mut term, &mut editor, &mut doc, |line| {
        eprintln!("{}", messages.event_line(line));
    }) {
        eprintln!("{}", messages.terminal_failed(&e.to_string()));
        std::process::exit(1);
    }
}

/// Открыть документ из командной строки или пустой, если файла нет.
fn open_document(
    editor: &Editor,
    path: Option<PathBuf>,
) -> Result<claxis::router::Document, SessionError> {
    match path {
        Some(path) => editor.session().open_file(path),
        None => editor.session().open_file(std::path::Path::new("")),
    }
}

use std::io::Write;
use std::path::PathBuf;

use claxis::api::term::Terminal;
use claxis::cli::{self, Command};
use claxis::main_messages::{En, Messages};
use claxis::messages::Messages as SessionMessages;
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
            if let Err(e) = cli::print_help(&mut out, &messages).and_then(|()| out.flush()) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        // Такого аргумента нет: молча открывать файл с именем `--что-то` хуже, чем
        // сказать, что такого имени нет.
        Command::Unknown { arg } => {
            eprintln!("{}", messages.unknown_argument(&arg));
            std::process::exit(2);
        }
        // Редактор открывается с терминалом, дальше идёт обычный цикл.
        Command::Run { path } => path,
    };

    run(path);
}

fn run(path: Option<PathBuf>) {
    let messages = En;
    // Редактор и документ собираются до терминала. Предупреждения о конфиге
    // и о чужой правке печатаются здесь же: в raw mode вывод летит мимо
    // позиционирования и остаётся в экране навсегда.
    let mut editor = match Editor::open() {
        Ok(e) => e,
        Err(e) => {
            // Текст ошибки собирает каталог, а не `Display`: у ошибок есть
            // структура, и текст для неё живёт в каталоге.
            eprintln!(
                "{}",
                messages.session_failed(&e.message(&claxis::messages::En))
            );
            std::process::exit(1);
        }
    };

    // О сломанном конфиге говорим сразу: молчать нельзя, иначе пользователь
    // не увидит, что его правка не сработала.
    if editor.used_last_good {
        eprintln!("{}", messages.event_line(&messages.using_last_good()));
    }
    for issue in editor.issues.clone() {
        eprintln!("{}", messages.event_line(&issue.text()));
    }
    // Папки, за которыми следить не вышло: без этого человек ждёт, что смена
    // темы применится сама, и не понимает, почему не применилась.
    for folder in &editor.blind_spots {
        eprintln!(
            "{}",
            messages.event_line(&claxis::messages::En.not_watched(folder))
        );
    }
    // Огрех схемы: значение по умолчанию не того вида. Работаем на дефолтах,
    // но сказать надо — иначе настройка «сама не применяется» останется
    // загадкой.
    if let Some(e) = &editor.schema_error {
        eprintln!("{}", messages.event_line(&e.text()));
    }

    // Файла может не быть: редактор с этим умеет, и пустой документ —
    // такой же документ, как остальные.
    let mut doc = match open_document(&editor, path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!(
                "{}",
                messages.session_failed(&e.message(&claxis::messages::En))
            );
            std::process::exit(1);
        }
    };
    // Файл могли поменять снаружи, пока редактор был закрыт. Снапшот тогда не
    // применён, и об этом честно сказано: молча пропавший текст хуже отшибка.
    if doc.changed_outside() {
        eprintln!(
            "{}",
            messages.event_line(&messages.changed_outside(&doc.path().display().to_string()))
        );
    }

    let mut term = match Terminal::new() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}", messages.terminal_failed(&e.to_string()));
            std::process::exit(1);
        }
    };

    let mut cycle = Cycle::new();
    let result = cycle.run(
        &mut term,
        &mut editor,
        &mut doc,
        &claxis::messages::En,
        |line| eprintln!("{}", messages.event_line(line)),
    );
    if let Err(e) = result {
        eprintln!("{}", messages.terminal_failed(&e.to_string()));
    }
    // Терминал закрываем сами, а не по `Drop`: если он остался в raw mode или
    // протокол не отключился, человек должен об этом узнать сейчас, а не
    // разгадывать потом, почему после редактора всё сломалось.
    if let Err(e) = term.close() {
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

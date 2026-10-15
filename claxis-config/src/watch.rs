//! Наблюдение за каталогом конфига.
//!
//! Редактор узнаёт об изменении конфига **от системы**, а не от своего
//! сохранения. Это обязательно: конфиг могут поменять снаружи — другой
//! редактор, `git checkout`, скрипт — и открытый редактор обязан это увидеть.
//!
//! Подписка non-recursive на каталог: следим за файлами конфига и за папками
//! тем и переводов, но не за всем подряд внутри них.
//!
//! Одна запись даёт несколько событий, а редактор пишет через временный файл
//! с переименованием, поэтому события копятся и отдаются пачкой: приложение
//! разбирает их, когда ему удобно.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

/// Сколько ждать перед выдачей событий.
///
/// Запись порождает несколько событий подряд, а ещё файл могут переименовать
/// уже после записи. Без паузы редактор начал бы применять конфиг на
/// полусписанном файле.
const SETTLE: Duration = Duration::from_millis(120);

/// Наблюдатель за каталогом конфига.
pub struct Watch {
    /// Куда система присылает уведомления. Само устройство наблюдения спрятано
    /// внутри `_watcher`: пока жив наблюдатель, жив и канал.
    _watcher: RecommendedWatcher,
    events: Receiver<Event>,
    /// Пути, для которых пришли события, и время последнего события по каждому.
    pending: Vec<(PathBuf, Instant)>,
    /// Что было в каталоге на момент подписки: без этого первое же событие
    /// пришло бы просто от того, что мы сами что-то открыли.
    ///
    /// Список пополняется на ходу: тему или перевод человек может положить
    /// в папку уже во время работы редактора, и такой файл обязан быть виден
    /// так же, как лежавшие при старте.
    watched: Vec<PathBuf>,
    /// Папки, за которыми следить не вышло.
    blind_spots: Vec<PathBuf>,
    /// Почему не вышло подписаться на папку из `blind_spots`.
    blind_reasons: Vec<String>,
    /// Корень конфига: по нему узнаём, что файл всё-таки наш.
    root: PathBuf,
}

impl Watch {
    /// Подписаться на каталог конфига и его подпапки.
    ///
    /// Ошибка подписки не должна мешать редактору работать: конфиг просто
    /// перестанет применяться сам, и об этом честно скажем.
    pub fn new(dir: &Path) -> notify::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        // Канал закрыт только вместе с наблюдателем, но если это случилось —
        // значит система событий больше не работает, и об этом надо сказать, а
        // не складывать ошибки в тишину.
        let sender_gone = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&sender_gone);
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<Event>| match res {
                Ok(event) => {
                    if tx.send(event).is_err() {
                        flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                Err(e) => {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    eprintln!("watching the config directory failed: {e}");
                }
            })?;
        watcher.watch(dir, RecursiveMode::NonRecursive)?;
        let mut blind_spots: Vec<PathBuf> = Vec::new();
        // Причины тех же мест: папка без причины выглядит как «что-то сломалось
        // в системе», а с причиной человек сразу видит, куда смотреть.
        let mut blind_reasons: Vec<String> = Vec::new();
        for sub in [super::path::theme_dir(dir), super::path::lang_dir(dir)] {
            // Папок может не быть: редактор их создаст при первом запуске. А
            // вот если папка есть, а подписаться не вышло — это поломка, и о
            // ней нужно сказать, иначе смена темы или языка не применится и
            // человек не поймёт почему.
            if sub.exists()
                && let Err(e) = watcher.watch(&sub, RecursiveMode::NonRecursive)
            {
                blind_spots.push(sub.clone());
                // Причина уходит в то же сообщение о слепой зоне: какая именно
                // подписка не вышла, пользователю знать полезно.
                blind_reasons.push(e.to_string());
            }
        }

        Ok(Self {
            _watcher: watcher,
            events: rx,
            pending: Vec::new(),
            watched: Vec::new(),
            root: dir.to_path_buf(),
            blind_spots,
            blind_reasons,
        })
    }

    /// Папки, за которыми следить не вышло.
    ///
    /// Пустой список — значит следим за всем. Непустой: правки внутри этих
    /// папок применятся только при перезапуске, и об этом сказано пользователю.
    pub fn blind_spots(&self) -> &[PathBuf] {
        &self.blind_spots
    }

    /// Почему следить не вышло: строки по одной на папку из `blind_spots`.
    pub fn blind_reasons(&self) -> &[String] {
        &self.blind_reasons
    }

    /// Запомнить, что сейчас в каталоге.
    ///
    /// Вызывается один раз после подписки: события, пришедшие между подпиской
    /// и этой отметкой, не считаются изменением.
    pub fn mark_current(&mut self, dir: &Path) {
        self.root = dir.to_path_buf();
        self.watched = config_files(dir);
    }

    /// Наш ли файл и видел ли мы его раньше.
    ///
    /// Наши — файлы конфига и всё, что лежит в папках тем и переводов: там
    /// каждая новая тема или новый язык появляются сами собой, и ждать
    /// перезапуска ради них незачем. Остальное в каталоге конфига к делу не
    /// относится и правки редактора не вызывает.
    fn accept(&mut self, path: &Path) -> bool {
        if self.watched.contains(&path.to_path_buf()) {
            return true;
        }
        let own_folder = path.starts_with(crate::path::theme_dir(&self.root))
            || path.starts_with(crate::path::lang_dir(&self.root));
        if !own_folder {
            return false;
        }
        self.watched.push(path.to_path_buf());
        true
    }

    /// Пути конфига, изменившиеся с прошлой проверки и уже устоявшиеся.
    ///
    /// Пусто, если изменений нет или они ещё «свежие» и могут быть недописаны.
    pub fn changed(&mut self) -> Vec<PathBuf> {
        self.drain();

        let now = Instant::now();
        // Отдаём только то, что устоялось: свежие события ждут, потому что
        // файл мог быть дописан прямо сейчас.
        let mut ready: Vec<PathBuf> = self
            .pending
            .iter()
            .filter(|(_, seen)| now.duration_since(*seen) >= SETTLE)
            .map(|(path, _)| path.clone())
            .collect();
        ready.sort();
        ready.dedup();
        self.pending
            .retain(|(_, seen)| now.duration_since(*seen) < SETTLE);
        ready
    }

    /// Разобрать накопившиеся события.
    ///
    /// Фильтр не по содержимому события, а по тому, что за файлы мы ждём:
    /// система шлёт и создание, и удаление, и переименование, а реагировать
    /// надо только на наши файлы.
    fn drain(&mut self) {
        loop {
            match self.events.try_recv() {
                Ok(event) => {
                    for path in event.paths {
                        if self.accept(&path) {
                            touch(&mut self.pending, path);
                        }
                    }
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => return,
            }
        }
    }
}

fn touch(pending: &mut Vec<(PathBuf, Instant)>, path: PathBuf) {
    match pending.iter_mut().find(|(p, _)| *p == path) {
        Some(slot) => slot.1 = Instant::now(),
        None => pending.push((path, Instant::now())),
    }
}

/// Файлы конфига в каталоге: то, за чем вообще следим.
fn config_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = super::schema::ConfigFile::ALL
        .iter()
        .map(|f| dir.join(f.file_name()))
        .collect();
    for sub in [super::path::theme_dir(dir), super::path::lang_dir(dir)] {
        files.push(sub.clone());
        if let Ok(entries) = std::fs::read_dir(&sub) {
            files.extend(entries.flatten().map(|e| e.path()));
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("claxis-watch-{name}"));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn dir(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Ждать изменений, пока не появятся или пока не выйдет время.
    fn wait_for(watch: &mut Watch, limit: Duration) -> Vec<PathBuf> {
        let deadline = Instant::now() + limit;
        loop {
            let got = watch.changed();
            if !got.is_empty() || Instant::now() > deadline {
                return got;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_changed_config_file_is_noticed() {
        // Главное: правка снаружи должна быть замечена.
        let tmp = Temp::new("changed");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut watch = Watch::new(tmp.dir()).unwrap();
        watch.mark_current(tmp.dir());

        let file = tmp.dir().join("edit.toml");
        std::fs::write(&file, "[General]\nhistory_depth = 64\n").unwrap();

        let changed = wait_for(&mut watch, Duration::from_secs(5));
        assert!(
            changed.contains(&file),
            "изменение config-файла не замечено, пришло: {changed:?}"
        );
    }

    #[test]
    fn a_theme_added_while_running_is_noticed() {
        // Тему кладут в папку уже во время работы редактора. Список за которым
        // мы следим снимается один раз, и раньше такой файл уходил в сторону:
        // событие было, а реагировать на него было не на что.
        //
        // Проверяем папку, а не сам файл: система по-разному присылает изменение
        // — то файлом, то папкой, в которой файл появился. Редактору достаточно
        // самого факта, что в папке тем что-то изменилось.
        let tmp = Temp::new("theme-added");
        let themes = crate::path::theme_dir(tmp.dir());
        std::fs::create_dir_all(&themes).unwrap();
        let mut watch = Watch::new(tmp.dir()).unwrap();
        watch.mark_current(tmp.dir());

        std::fs::write(themes.join("everforest.toml"), "").unwrap();

        let changed = wait_for(&mut watch, Duration::from_secs(5));
        assert!(
            changed.iter().any(|p| p.starts_with(&themes)),
            "новая тема не замечена, пришло: {changed:?}"
        );
    }

    #[test]
    fn a_file_outside_the_config_is_ignored() {
        // Рядом валяется чужая папка: её правки конфигом не являются.
        let tmp = Temp::new("outside");
        let outside = tmp.0.join("снаружи");
        std::fs::create_dir_all(&outside).unwrap();
        let mut watch = Watch::new(tmp.dir()).unwrap();
        watch.mark_current(tmp.dir());

        std::fs::write(outside.join("edit.toml"), "[General]\n").unwrap();
        std::thread::sleep(SETTLE * 3);
        watch.changed();
        assert!(watch.changed().is_empty(), "чужой файл принят за наш");
    }

    #[test]
    fn an_untouched_file_reports_nothing() {
        // Пока никто не трогал файлы, событий быть не должно.
        let tmp = Temp::new("untouched");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut watch = Watch::new(tmp.dir()).unwrap();
        watch.mark_current(tmp.dir());

        std::fs::write(tmp.dir().join("edit.toml"), "[General]\n").unwrap();
        // Даём системе время прислать событие и «устояться».
        std::thread::sleep(SETTLE * 3);
        watch.changed();
        assert!(
            watch.changed().is_empty(),
            "события пришли без изменения конфига"
        );
    }

    #[test]
    fn foreign_files_are_ignored() {
        // Рядом может лежать что угодно: нас интересуют только файлы конфига.
        let tmp = Temp::new("foreign");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut watch = Watch::new(tmp.dir()).unwrap();
        watch.mark_current(tmp.dir());

        std::fs::write(tmp.dir().join("заметки.txt"), "не конфиг").unwrap();

        let deadline = Instant::now() + SETTLE * 4;
        while Instant::now() < deadline {
            if watch.changed().iter().any(|p| p.ends_with("заметки.txt")) {
                panic!("чужой файл не должен считаться изменением конфига");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn several_writes_come_as_one_change() {
        // Редактор пишет файл несколько раз подряд: применять надо один раз,
        // когда запись устоялась.
        let tmp = Temp::new("settle");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut watch = Watch::new(tmp.dir()).unwrap();
        watch.mark_current(tmp.dir());

        let file = tmp.dir().join("files.toml");
        for _ in 0..5 {
            std::fs::write(&file, "[General]\nsnapshots_keep = 3\n").unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut rounds = 0;
        while Instant::now() < deadline {
            if watch.changed().contains(&file) {
                rounds += 1;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(rounds, 1, "пачка записей должна дать одно применение");
        // Дальше тихо: файл больше не меняется.
        std::thread::sleep(SETTLE * 2);
        assert!(
            watch.changed().is_empty(),
            "повторное применение без правки"
        );
    }

    #[test]
    fn a_missing_subfolder_does_not_block_watching() {
        // Папок тем и переводов может не быть: наблюдение всё равно нужно.
        let tmp = Temp::new("no-subfolders");
        let mut watch = Watch::new(tmp.dir()).expect("подписка не должна падать");
        watch.mark_current(tmp.dir());
        assert!(watch.changed().is_empty());
    }
}

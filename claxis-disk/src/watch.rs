//! Наблюдение за файлами.
//!
//! Редактор узнаёт об изменении файла **от системы**, а не от своего
//! сохранения: файлы меняют снаружи — другой редактор, `git checkout`,
//! скрипт — и открытый редактор обязан это увидеть.
//!
//! Зависимость `notify` живёт здесь и наружу не выходит: она нужна может быть
//! многим, а доступна не всем.
//!
//! Крейт не решает, чьи это файлы. Он говорит, что изменилось под корнем, а
//! что из этого «своё» — знает уже вызывающий.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::error::Error;

/// Сколько ждать перед выдачей событий.
///
/// Одна запись порождает несколько событий подряд, а ещё файл могут
/// переименовать уже после записи. Без паузы вызывающий начал бы читать
/// наполовину записанный файл.
const SETTLE: Duration = Duration::from_millis(120);

/// Папка, за которой следить не вышло, и почему.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlindSpot {
    /// Каталог, который не удалось подписать.
    pub dir: PathBuf,
    /// Причина отказа.
    pub reason: String,
}

/// Что изменилось под наблюдаемым корнем.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    /// Пути, изменившиеся и уже устоявшиеся.
    pub paths: Vec<PathBuf>,
    /// Ошибки, которые прислала система наблюдения.
    ///
    /// Наблюдение молча перестаёт работать незаметно для человека: правка
    /// перестаёт применяться, а выглядит всё так же. Молчать здесь нельзя.
    pub failures: Vec<String>,
}

impl Changed {
    /// Изменилось ли что-то за это время.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.failures.is_empty()
    }
}

/// Наблюдатель за каталогом.
pub struct Watch {
    /// Само устройство наблюдения: пока жив наблюдатель, жив и канал.
    ///
    /// Поле не читается, но и не мёртвое: оно держит подписку. `Debug`
    /// вручную: самого наблюдателя печатать незачем, а ошибки тестов любят
    /// его называть.
    #[expect(dead_code, reason = "держит подписку живой")]
    watcher: RecommendedWatcher,
    events: Receiver<Event>,
    /// Пути, для которых пришли события, и время последнего события по
    /// каждому.
    pending: Vec<(PathBuf, Instant)>,
    /// Папки, за которыми следить не вышло.
    blind_spots: Vec<BlindSpot>,
    /// Ошибки системы наблюдения: приходят из другого потока, поэтому через
    /// общий ящик, и забирает их тот, кто спрашивает.
    reported: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// Корень наблюдения: всё вне его не наше, даже если система прислала.
    root: PathBuf,
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watch")
            .field("root", &self.root)
            .field("pending", &self.pending.len())
            .field("blind_spots", &self.blind_spots)
            .finish()
    }
}

impl Watch {
    /// Подписаться на каталог и на перечисленные подкаталоги.
    ///
    /// Отсутствующие подкаталоги пропускаются: их может не быть, и это не
    /// поломка. Не выходит подписаться только на существующий — тогда он
    /// попадает в `blind_spots`, и вызывающий говорит об этом пользователю.
    pub fn new(root: &Path, subdirs: &[PathBuf]) -> crate::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        // Ошибки системы наблюдения уходят в список, а не в молчание: молчащее
        // наблюдение хуже явного отказа.
        let failures: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&failures);

        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<Event>| match res {
                Ok(event) => {
                    if tx.send(event).is_err() {
                        // Канал закрыт вместе с наблюдателем; события больше
                        // некуда деть, и это надо сказать, а не складывать в
                        // тишину.
                        report(&sink, "the watcher is no longer running");
                    }
                }
                Err(e) => report(&sink, e.to_string()),
            })
            .map_err(|e| Error::Watch {
                path: root.display().to_string(),
                reason: e.to_string(),
            })?;

        watcher
            .watch(root, RecursiveMode::NonRecursive)
            .map_err(|e| Error::Watch {
                path: root.display().to_string(),
                reason: e.to_string(),
            })?;

        let mut blind_spots = Vec::new();
        for sub in subdirs {
            // Папки может не быть: её создадут позже. А вот если папка есть,
            // а подписаться не вышло — это поломка, и о ней нужно сказать.
            if !sub.exists() {
                continue;
            }
            if let Err(e) = watcher.watch(sub, RecursiveMode::NonRecursive) {
                blind_spots.push(BlindSpot {
                    dir: sub.clone(),
                    reason: e.to_string(),
                });
            }
        }

        Ok(Self {
            watcher,
            events: rx,
            pending: Vec::new(),
            blind_spots,
            reported: failures,
            root: root.to_path_buf(),
        })
    }

    /// Папки, за которыми следить не вышло.
    pub fn blind_spots(&self) -> &[BlindSpot] {
        &self.blind_spots
    }

    /// Что изменилось и что сломалось в самом наблюдении.
    ///
    /// Отдаётся только то, что устоялось: свежие события ждут, потому что
    /// файл мог быть дописан прямо сейчас.
    pub fn changed(&mut self) -> Changed {
        self.drain();

        let now = Instant::now();
        let mut paths: Vec<PathBuf> = self
            .pending
            .iter()
            .filter(|(_, seen)| now.duration_since(*seen) >= SETTLE)
            .map(|(path, _)| path.clone())
            .collect();
        paths.sort();
        paths.dedup();
        self.pending
            .retain(|(_, seen)| now.duration_since(*seen) < SETTLE);

        // Ошибки системы забираются один раз: показаны — и всё.
        let failures = match self.reported.lock() {
            Ok(mut shared) => std::mem::take(&mut *shared),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        };

        Changed { paths, failures }
    }

    /// Разобрать накопившиеся события.
    ///
    /// Фильтр не по содержимому события, а по тому, что файл лежит под нашим
    /// корнем: система шлёт и создание, и удаление, и переименование, а чьи
    /// это файлы — решает вызывающий.
    fn drain(&mut self) {
        loop {
            match self.events.try_recv() {
                Ok(event) => {
                    for path in event.paths {
                        if path.starts_with(&self.root) {
                            touch(&mut self.pending, path);
                        }
                    }
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return,
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

/// Отложить ошибку системы наблюдения до того, как её спросят.
fn report(sink: &std::sync::Arc<std::sync::Mutex<Vec<String>>>, reason: impl Into<String>) {
    // Ящик мог быть отравлен паникой в другом потоке: терять из-за этого
    // ошибку наблюдения нельзя, поэтому забираем данные и при отравленном.
    match sink.lock() {
        Ok(mut shared) => shared.push(reason.into()),
        Err(poisoned) => poisoned.into_inner().push(reason.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::write_text;

    /// Каталог под тест, удаляемый вместе с собой.
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("claxis-disk-watch-{name}"));
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
    fn wait_for(watch: &mut Watch, limit: Duration) -> Changed {
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
    fn a_changed_file_is_noticed() {
        // Главное: правка снаружи должна быть замечена.
        let tmp = Temp::new("changed");
        let mut watch = Watch::new(tmp.dir(), &[]).unwrap();

        let file = tmp.dir().join("edit.toml");
        std::fs::write(&file, "[General]\nhistory_depth = 64\n").unwrap();

        let got = wait_for(&mut watch, Duration::from_secs(5));
        assert!(
            got.paths.contains(&file),
            "изменение не замечено, пришло: {:?}",
            got.paths
        );
    }

    #[test]
    fn a_file_added_later_is_noticed_too() {
        // Список слежения снимается один раз, и раньше новый файл уходил в
        // сторону: событие было, а отвечать было не на что.
        let tmp = Temp::new("added");
        let mut watch = Watch::new(tmp.dir(), &[]).unwrap();

        let file = tmp.dir().join("files.toml");
        write_text(&file, "[General]\n").unwrap();

        let got = wait_for(&mut watch, Duration::from_secs(5));
        assert!(got.paths.contains(&file), "пришло: {:?}", got.paths);
    }

    #[test]
    fn several_writes_come_as_one_change() {
        // Редактор пишет файл несколько раз подряд: применять надо один раз,
        // когда запись устоялась.
        let tmp = Temp::new("settle");
        let mut watch = Watch::new(tmp.dir(), &[]).unwrap();

        let file = tmp.dir().join("files.toml");
        for i in 0..5 {
            write_text(&file, &format!("[General]\nsnapshots_keep = {i}\n")).unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut rounds = 0;
        while Instant::now() < deadline {
            if watch.changed().paths.contains(&file) {
                rounds += 1;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(rounds, 1, "пачка записей должна дать одно применение");

        // Дальше тихо: файл больше не меняется.
        std::thread::sleep(SETTLE * 2);
        assert!(
            watch.changed().paths.is_empty(),
            "повторное применение без правки"
        );
    }

    #[test]
    fn a_subdirectory_is_watched_too() {
        // Темы и переводы лежат в подпапках, и они тоже должны быть под
        // наблюдением.
        let tmp = Temp::new("subdir");
        let sub = tmp.dir().join("theme");
        std::fs::create_dir_all(&sub).unwrap();
        let mut watch = Watch::new(tmp.dir(), std::slice::from_ref(&sub)).unwrap();

        let file = sub.join("everforest.toml");
        write_text(&file, "").unwrap();

        let got = wait_for(&mut watch, Duration::from_secs(5));
        assert!(
            got.paths.iter().any(|p| p.starts_with(&sub)),
            "подпапка не наблюдается, пришло: {:?}",
            got.paths
        );
    }

    #[test]
    fn a_missing_subdirectory_is_not_a_failure() {
        // Папки тем может не быть: наблюдение всё равно нужно.
        let tmp = Temp::new("no-subdir");
        let missing = tmp.dir().join("lang");
        let watch = Watch::new(tmp.dir(), &[missing]).expect("подписка не должна падать");
        assert!(watch.blind_spots().is_empty(), "нет папки — не слепая зона");
    }

    #[test]
    fn a_watch_that_cannot_start_says_where() {
        // Каталога нет: подписка не выходит, и человек должен знать где именно.
        let tmp = Temp::new("no-root");
        let missing = tmp.dir().join("нет-такого");
        let error = Watch::new(&missing, &[]).unwrap_err();
        assert!(error.text().contains("нет-такого"), "{error}");
    }

    #[test]
    fn a_file_outside_the_root_is_ignored() {
        // Подписка non-recursive: то, что рядом с корнем, но не внутри,
        // наблюдением не является.
        let tmp = Temp::new("outside");
        let mut watch = Watch::new(tmp.dir(), &[]).unwrap();

        let sibling = tmp
            .dir()
            .parent()
            .unwrap()
            .join("claxis-disk-watch-sibling");
        std::fs::create_dir_all(&sibling).unwrap();
        write_text(&sibling.join("посторонний.toml"), "x").unwrap();
        std::thread::sleep(SETTLE * 3);

        assert!(
            !watch
                .changed()
                .paths
                .iter()
                .any(|p| p.starts_with(&sibling)),
            "чужой файл принят за наш"
        );
        let _ = std::fs::remove_dir_all(&sibling);
    }

    #[test]
    fn an_untouched_directory_reports_nothing() {
        // Пока никто не трогал файлы, событий быть не должно.
        let tmp = Temp::new("untouched");
        let mut watch = Watch::new(tmp.dir(), &[]).unwrap();

        std::thread::sleep(SETTLE * 3);
        assert!(watch.changed().is_empty(), "события без изменения");
    }

    #[test]
    fn nothing_changed_means_an_empty_answer() {
        let tmp = Temp::new("empty-answer");
        let mut watch = Watch::new(tmp.dir(), &[]).unwrap();
        let got = watch.changed();
        assert!(got.is_empty());
        assert!(got.paths.is_empty());
        assert!(got.failures.is_empty());
    }
}

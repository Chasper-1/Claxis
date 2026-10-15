//! Связывает документ с хранилищем снапшотов.
//!
//! Оба крейта по отдельности ничего не знают друг о друге: `claxis-rope`
//! отдаёт текст и историю приёмнику и не знает, куда они уйдут,
//! `claxis-store` умеет только складывать и доставать байты. Соединяются они
//! здесь — это и есть работа маршрутизатора.

use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use crate::api::rope::defaults::buffer::HISTORY_DEPTH;
use crate::api::rope::{
    Document as Rope, Error as RopeError, History, MIN_DEPTH, Record as RopeRecord, SnapshotSink,
};
use crate::api::store::{KEEP, PERSIST, SnapshotStore, StorePaths};
use crate::messages::Messages;
use crate::router::lines::LineMap;
use crate::router::stats::{self, Stats};

/// Что пошло не так в сессии.
#[derive(Debug)]
pub enum SessionError {
    /// Документ не создан: глубина истории недопустима.
    Buffer(crate::api::rope::Error),
    /// Хранилище не открылось.
    Store { path: String, reason: String },
    /// Файл документа не прочитан.
    ReadFile { path: String, reason: String },
    /// Файл больше, чем помещается в `u32`.
    FileTooLarge { bytes: u64 },
}

impl SessionError {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    pub fn message(&self, messages: &dyn Messages) -> String {
        match self {
            SessionError::Buffer(e) => messages.buffer_error(e),
            SessionError::Store { path, reason } => messages.store_failed(path, reason),
            SessionError::ReadFile { path, reason } => messages.read_file_failed(path, reason),
            SessionError::FileTooLarge { bytes } => messages.file_too_large(
                *bytes,
                *bytes as f64 / 1_000_000_000.0,
                u32::MAX as u64,
                u32::MAX as f64 / 1_000_000_000.0,
            ),
        }
    }

    /// Текст ошибки на языке по умолчанию.
    pub fn text(&self) -> String {
        self.message(&crate::messages::En)
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::error::Error for SessionError {}

/// Приёмник снапшота, пишущий в хранилище.
///
/// Это тот самый стык: документ отдаёт текст и историю, дальше они уходят в
/// sqlite байтами, в том формате, в котором их собрал rope.
#[derive(Debug)]
struct StoreSink {
    store: SnapshotStore,
    /// Последняя ошибка записи: снапшот не лёг, и об этом должен узнать человек.
    ///
    /// Ошибку нельзя вернуть из `accept`: у приёмника нет вызывающего, а
    /// разорвать из-за неё правку нельзя — текст уже изменён. Поэтому ошибка
    /// кладётся в общую ячейку, документ забирает её после снятия снапшота и
    /// говорит о ней пользователю. Молчаливая потеря работы хуже всего.
    failed: Rc<RefCell<Option<String>>>,
}

impl SnapshotSink for StoreSink {
    fn accept(&mut self, file: &Path, text: &[u8], history: &[u8]) {
        // История идёт вместе с текстом: без неё после перезапуска отменять
        // было бы нечего, то есть снапшот сохранил бы половину работы.
        if let Err(e) = self.store.put(file, text, history) {
            *self.failed.borrow_mut() = Some(e.text());
        }
    }
}

/// Источник времени.
///
/// Отдельный тип, а не `Instant::now()` в коде: иначе проверить, что сработало
/// правильно, можно было бы только ожиданием вживую.
pub trait Clock: std::fmt::Debug {
    /// Сколько прошло с начала работы процесса.
    fn elapsed(&self) -> Duration;
}

/// Часы, которые двигает тест: ждать настоящие пять минут нельзя.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock(pub Duration);

impl FixedClock {
    /// Часы, прошедшие указанное время от старта процесса.
    pub fn after(elapsed: Duration) -> Self {
        Self(elapsed)
    }
}

impl Clock for FixedClock {
    fn elapsed(&self) -> Duration {
        self.0
    }
}

/// Настоящие часы.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn elapsed(&self) -> Duration {
        use std::sync::OnceLock;
        use std::time::Instant;
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed()
    }
}

/// Открытый документ: rope плюс привязка к файлу.
///
/// Время живёт здесь, а не в документе: снапшот — это решение сессии, у неё
/// есть и документ, и хранилище. Сам документ про часы не знает и снимает
/// снапшот только по прямой просьбе.
#[derive(Debug)]
pub struct Document {
    path: PathBuf,
    buffer: Rope,
    /// Через сколько снимать следующий снапшот.
    interval: Duration,
    /// Сколько прошло с начала работы, когда снят последний снапшот.
    last_snapshot: Duration,
    /// Откуда берётся время.
    clock: Box<dyn Clock>,
    /// Что набрано за сеанс работы над этим документом.
    stats: Stats,
    /// Где в документе переводы строк.
    ///
    /// Карта живёт здесь, а не в буфере: документ знает про байты и ничего не
    /// знает про строки. Сюда приходят только дельты правок.
    lines: LineMap,
    /// Файл на диске разошёлся с тем, на котором строился снапшот.
    ///
    /// Снапшот тогда не применяется: иначе правка, сделанная снаружи, была бы
    /// не видна и уехала бы под следующим сохранением. Пользователю об этом
    /// говорит вызывающий.
    changed_outside: bool,
    /// Ячейка, куда приёмник кладёт ошибку записи снапшота.
    ///
    /// Приёмник живёт внутри rope и обратного хода не имеет: ошибку забирает
    /// документ при следующем снятии снапшота и показывает пользователю.
    sink_failed: Rc<RefCell<Option<String>>>,
    /// Ошибка записи снапшота, ждущая показа.
    snapshot_failed: Option<String>,
}

impl Document {
    /// Путь к файлу.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Документ для чтения: текст, история, длина.
    ///
    /// Правок тут нет намеренно: иначе можно изменить документ мимо карты
    /// строк и статистики, и они тихо разъедутся.
    pub fn buffer(&self) -> &Rope {
        &self.buffer
    }

    /// Текст документа.
    pub fn text(&self) -> Vec<u8> {
        self.buffer.read()
    }

    /// Карта строк документа.
    pub fn lines(&self) -> &LineMap {
        &self.lines
    }

    /// Файл меняли снаружи, пока редактор был закрыт.
    ///
    /// Снапшот тогда не применялся: иначе правка извне была бы не видна и
    /// уехала бы под следующим сохранением. Документ открыт как есть, и об
    /// этом честно сказано.
    pub fn changed_outside(&self) -> bool {
        self.changed_outside
    }

    /// Номер строки для позиции, начиная с нуля.
    pub fn line_of(&self, pos: u32) -> u32 {
        self.lines.line_of(pos)
    }

    /// Сколько строк в документе.
    pub fn line_count(&self) -> u32 {
        self.lines.line_count()
    }

    /// Позиция, с которой начинается строка.
    pub fn offset_of_line(&self, line: u32) -> u32 {
        self.lines.offset_of_line(line, self.buffer.len())
    }

    /// Позиция конца строки: сам перевод строки.
    pub fn end_of_line(&self, line: u32) -> u32 {
        self.lines.end_of_line(line, self.buffer.len())
    }

    /// Что собрано за сеанс: символы, нажатия, правки.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Вставить текст и посчитать введённые символы.
    pub fn insert(&mut self, anchor: u32, data: &[u8]) -> Result<(), RopeError> {
        self.buffer.insert(anchor, data)?;
        self.lines.insert(anchor, data);
        stats::note_input(&mut self.stats, data);
        Ok(())
    }

    /// Вырезать диапазон и посчитать удалённые символы.
    pub fn delete(&mut self, anchor: u32, len: u32) -> Result<(), RopeError> {
        self.buffer.delete(anchor, len)?;
        self.lines.delete(anchor, anchor + len);
        stats::note_deleted(&mut self.stats, len);
        Ok(())
    }

    /// Заменить диапазон: вставка и удаление одной операцией.
    pub fn replace(&mut self, anchor: u32, len: u32, data: &[u8]) -> Result<(), RopeError> {
        self.buffer.replace(anchor, len, data)?;
        self.replace_in_lines(anchor, len, data);
        stats::note_replace(&mut self.stats, data, len);
        Ok(())
    }

    /// Провести карту строк по правке: диапазон исчез, на его место пришёл текст.
    ///
    /// Один ход на оба действия, а не два вызова карты: между ними не должно
    /// быть места, где документ уже изменён, а карта ещё нет.
    fn replace_in_lines(&mut self, anchor: u32, len: u32, data: &[u8]) {
        if len > 0 {
            self.lines.delete(anchor, anchor + len);
        }
        self.lines.insert(anchor, data);
    }

    /// Отменить правку. Карта строк едет вместе с текстом.
    pub fn undo(&mut self) -> Result<Option<u32>, RopeError> {
        let path = self.buffer.path();
        let Some(id) = self.buffer.undo()? else {
            return Ok(None);
        };
        // Запись узнаём по её месту в ветке: отмена убрала последнюю.
        if let Some(record) = path.get(path.len().wrapping_sub(1)) {
            self.undo_line(record);
        }
        Ok(Some(id))
    }

    /// Откатить карту строк под обратную операцию.
    fn undo_line(&mut self, record: &RopeRecord) {
        match record {
            // Отмена вставки: вырезать вставленное.
            RopeRecord::Insert { anchor, head, .. } => self.lines.delete(*anchor, *head),
            // Отмена удаления: вернуть вырезанное.
            RopeRecord::Delete { anchor, .. } => self.restore_lines(*anchor, record),
            // Отмена замены: вырезать вставленное и вернуть вырезанное.
            RopeRecord::Replace { anchor, head, .. } => {
                if *head > *anchor {
                    self.lines.delete(*anchor, *head);
                }
                self.restore_lines(*anchor, record);
            }
        }
    }

    /// Вернуть в карту текст, который запись вырезала из документа.
    ///
    /// Смещение берётся из самой записи: после отмены на прежнем месте может
    /// стоять совсем другое. Нет текста — значит отменять было нечего: пустая
    /// замена оставляет карту как есть.
    fn restore_lines(&mut self, anchor: u32, record: &RopeRecord) {
        let Some(removed) = self.buffer.removed_text(record) else {
            return;
        };
        let removed = removed.to_vec();
        self.lines.insert(anchor, &removed);
    }

    /// Повторить отменённую правку. Карта строк едет вместе с текстом.
    pub fn redo(&mut self) -> Result<Option<u32>, RopeError> {
        let before = self.buffer.path().len();
        let Some(id) = self.buffer.redo()? else {
            return Ok(None);
        };
        // Повтор добавил одну запись в конец ветки.
        if let Some(record) = self.buffer.path().get(before) {
            self.redo_line(record);
        }
        Ok(Some(id))
    }

    /// Провести карту строк по прямой правке.
    fn redo_line(&mut self, record: &RopeRecord) {
        match record {
            RopeRecord::Insert { anchor, .. } => {
                if let Some(piece) = self.buffer.inserted_text(record) {
                    let piece = piece.to_vec();
                    self.lines.insert(*anchor, &piece);
                }
            }
            RopeRecord::Delete { anchor, head, .. } => self.lines.delete(*anchor, *head),
            RopeRecord::Replace { anchor, head, .. } => {
                self.lines.delete(*anchor, *head);
                if let Some(piece) = self.buffer.inserted_text(record) {
                    let piece = piece.to_vec();
                    self.lines.insert(*anchor, &piece);
                }
            }
        }
    }

    /// Записать одно нажатие.
    ///
    /// Время нажатия дописывается отдельно: оно известно только в момент
    /// отпускания, а нажатие без отпускания — обычное дело.
    pub fn note_press(&mut self) {
        stats::note_press(&mut self.stats);
    }

    /// Дописать время, которое нажатие пролежало.
    pub fn note_held(&mut self, held: Duration) {
        stats::note_held(&mut self.stats, held);
    }

    /// Через сколько после последнего снапшота снимать следующий.
    pub fn snapshot_interval(&self) -> Duration {
        self.interval
    }

    /// Подменить часы: тестам нужно время, которое они двигают сами.
    ///
    /// Отсчёт не сбрасывается: иначе перевести часы вперёд было бы нельзя —
    /// документ решил бы, что только что снял снапшот.
    pub fn set_clock(&mut self, clock: Box<dyn Clock>) {
        self.clock = clock;
    }

    /// Снять снапшот, если с последнего прошло достаточно времени.
    ///
    /// Время, а не число правок: человек может править один и тот же абзац
    /// десять минут, и снапшот каждые пять минут отдаст память обратно. И
    /// наоборот — сто правок подряд за минуту снапшот не заслужили.
    pub fn snapshot_if_due(&mut self) {
        if self.clock.elapsed().saturating_sub(self.last_snapshot) >= self.interval {
            self.snapshot();
        }
    }

    /// Снять снапшот и отсчитать время заново.
    pub fn snapshot(&mut self) {
        // Текст от этого не меняется, поэтому карта строк остаётся в силе.
        self.buffer.snapshot();
        // Приёмник отдал текст и историю и ошибться мог там, где у него нет
        // вызывающего. Ошибку забираем здесь: она про работу пользователя, а не
        // про кеш, и молчать о ней нельзя.
        if let Some(reason) = self.sink_failed.borrow_mut().take() {
            self.snapshot_failed = Some(reason);
        }
        self.last_snapshot = self.clock.elapsed();
    }

    /// Снапшот не лёг на диск. Текст с причиной, уже переведённый.
    ///
    /// Пока ошибка не забрана вызывающим, она здесь: следующий вызов вернёт её
    /// и очистит, чтобы одно и то же не сообщать дважды.
    pub fn snapshot_error(&mut self) -> Option<String> {
        self.snapshot_failed.take()
    }
}

/// Снапшот по умолчанию: раз в пять минут.
pub const DEFAULT_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(300);

/// Редакторская сессия: хранилище снапшотов и открытые документы.
#[derive(Debug)]
pub struct Session {
    store: SnapshotStore,
    paths: StorePaths,
    /// Сколько записей правки живёт в памяти до снапшота. Из конфига.
    history_depth: u32,
    /// Писать ли снапшоты на диск. Из конфига.
    persist: bool,
    /// Через сколько снимать следующий снапшот. Из конфига.
    interval: Duration,
    /// Глубина, с которой сессия открыта и под которую созданы буферы.
    ///
    /// Отличается от `history_depth`, если конфиг поменяли на лету: буферы
    /// уже созданы под прежнюю, и пересоздать их без потери истории нельзя.
    opened_depth: u32,
}

/// Что применилось сразу, а что ждёт перезапуска.
///
/// Не всё настраивается на лету: размер пула узлов задан при создании буфера,
/// и сменить его можно только пересоздав буфер вместе с историей. Лучше сказать
/// пользователю, что нужен перезапуск, чем применить наполовину и сделать
/// вид, что всё в порядке.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// Ключи, подхваченные сразу.
    pub applied: Vec<&'static str>,
    /// Ключи, которые заработают после перезапуска.
    pub needs_restart: Vec<&'static str>,
}

impl Session {
    /// Открыть сессию по настройкам по умолчанию.
    ///
    /// Значения берутся из настроек крейтов: `keep` и `persist` — из стора,
    /// глубина истории — из буфера. Когда появится конфиг, он подставит сюда
    /// свои значения, и менять тут ничего не придётся.
    pub fn open_default() -> Result<Self, SessionError> {
        Self::open(KEEP, PERSIST, HISTORY_DEPTH.value)
    }

    /// Открыть сессию с заданными `keep` и глубиной истории.
    pub fn open(keep: u8, persist: bool, history_depth: u32) -> Result<Self, SessionError> {
        let paths = StorePaths::from_env();
        Self::build(paths, keep, persist, history_depth)
    }

    fn build(
        paths: StorePaths,
        keep: u8,
        persist: bool,
        history_depth: u32,
    ) -> Result<Self, SessionError> {
        let store = SnapshotStore::open(&paths, keep).map_err(|e| SessionError::Store {
            path: paths.database().display().to_string(),
            reason: e.to_string(),
        })?;
        Ok(Self {
            store,
            paths,
            history_depth,
            persist,
            interval: DEFAULT_SNAPSHOT_INTERVAL,
            opened_depth: history_depth,
        })
    }

    /// Открыть сессию с хранилищем в заданном каталоге — нужно в тестах.
    pub fn open_at(dir: impl Into<PathBuf>, keep: u8) -> Result<Self, SessionError> {
        Self::open_at_with(dir, keep, PERSIST, HISTORY_DEPTH.value)
    }

    /// То же, но с явными `persist` и глубиной истории.
    pub fn open_at_with(
        dir: impl Into<PathBuf>,
        keep: u8,
        persist: bool,
        history_depth: u32,
    ) -> Result<Self, SessionError> {
        Self::build(StorePaths::new(dir), keep, persist, history_depth)
    }

    /// То же, но путь принимается как ссылка.
    pub fn open_at_ref(
        dir: &Path,
        keep: u8,
        persist: bool,
        history_depth: u32,
    ) -> Result<Self, SessionError> {
        Self::open_at_with(dir.to_path_buf(), keep, persist, history_depth)
    }

    /// Сколько снапшотов одного файла хранится. Ноль — только память.
    pub fn keep(&self) -> u8 {
        self.store.keep()
    }

    /// Пишутся ли снапшоты на диск.
    pub fn persist(&self) -> bool {
        self.persist
    }

    /// Через сколько после последнего снапшота снимать следующий.
    pub fn snapshot_interval(&self) -> Duration {
        self.interval
    }

    /// Задать интервал между снапшотами. Применяется к документам, которые
    /// откроются дальше; уже открытые держат то, что получили при открытии.
    pub fn set_snapshot_interval(&mut self, interval: Duration) {
        self.interval = interval;
    }

    /// Глубина истории из последнего прочитанного конфига.
    pub fn history_depth(&self) -> u32 {
        self.history_depth
    }

    /// Глубина, под которую реально созданы буферы.
    ///
    /// Расходится с `history_depth`, если конфиг поменяли на лету: применять
    /// такой ключ можно только перезапуском.
    pub fn opened_depth(&self) -> u32 {
        self.opened_depth
    }

    /// Глубина для новых документов.
    ///
    /// Конфиг может назвать ноль, а буфер с таким значением не создаётся:
    /// редактор обязан сказать об этом ошибкой, а не приводить значение
    /// молча — человек у себя в конфиге такого не писал.
    fn history_depth_for_new_documents(&self) -> Result<u32, SessionError> {
        if self.history_depth < MIN_DEPTH {
            return Err(SessionError::Buffer(RopeError::InvalidHistoryDepth {
                depth: self.history_depth,
            }));
        }
        Ok(self.history_depth)
    }

    /// Где лежит хранилище.
    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    /// Хранилище снапшотов.
    pub fn store(&self) -> &SnapshotStore {
        &self.store
    }

    /// Применить настройки без перезапуска редактора.
    ///
    /// Конфиг сохранили — редактор перечитал его и применяет. Не всё можно
    /// применить на лету, и сказать об этом лучше, чем применить наполовину.
    ///
    /// `snapshots_keep` применяется сразу: это число вокруг которого
    /// выбираются снапшоты. `snapshots_persist` — сразу для документов,
    /// которые откроются дальше; уже открытые держат то, что получили при
    /// открытии. `history_depth` задаёт размер пула узлов при создании буфера
    /// и без пересоздания истории не меняется.
    pub fn apply(&mut self, settings: &crate::router::Settings) -> Applied {
        let mut applied = Applied::default();

        if self.store.keep() != settings.snapshots_keep {
            self.store.set_keep(settings.snapshots_keep);
            applied.applied.push("snapshots_keep");
        }
        if self.persist != settings.snapshots_persist {
            self.persist = settings.snapshots_persist;
            applied.applied.push("snapshots_persist");
        }
        if self.history_depth != settings.history_depth {
            self.history_depth = settings.history_depth;
            applied.needs_restart.push("history_depth");
        }

        applied
    }

    /// Открыть документ: прочитать файл и привязать снапшоты к хранилищу.
    ///
    /// Файла может не быть — тогда это новый пустой документ, ошибка не
    /// возникает.
    pub fn open_file(&self, path: impl AsRef<Path>) -> Result<Document, SessionError> {
        let path = path.as_ref().to_path_buf();

        // Проверяем размер до чтения: буфер работает с u32, файл больше
        // u32::MAX байт не помещается.
        if let Ok(metadata) = std::fs::metadata(&path) {
            let len = metadata.len();
            if len > u32::MAX as u64 {
                return Err(SessionError::FileTooLarge { bytes: len });
            }
        }

        let text = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                return Err(SessionError::ReadFile {
                    path: path.display().to_string(),
                    reason: e.to_string(),
                });
            }
        };
        let mut buffer = Rope::with_depth(text.clone(), self.history_depth_for_new_documents()?)
            .map_err(SessionError::Buffer)?;
        let mut changed_outside = false;
        // Снапшот под документом копит не только текст, но и историю правок.
        // Восстанавливаем её: после перезапуска отменять должно быть можно,
        // иначе половина работы пользователя просто исчезает.
        if self.persist
            && let Ok(store) = self.store.reopen()
            && let Ok(Some(snapshot)) = store.latest(&path)
        {
            // Чужой или битый блоб историю не восстановит — и не должен:
            // текст-то из базы верный, терять его из-за испорченного хвоста
            // незачем.
            if let Ok(history) = History::decode(&snapshot.history) {
                match history.base() {
                    // Файл тот же, на котором снят снапшот: применяем.
                    Ok(base) if base == text.as_slice() => {
                        if let Ok(restored) = history.restore() {
                            buffer = restored;
                        }
                    }
                    // Файл могли поменять снаружи. Применять снапшот нельзя:
                    // правка извне станет невидимой и уедет под следующим
                    // сохранением. Открываем файл как есть и говорим об этом.
                    Ok(_) => changed_outside = true,
                    Err(_) => {}
                }
            }
        }
        // Приёмник вешается только если снапшоты положено хранить. Ноль в кольце —
        // это «только память»: приёмника не будет вовсе, иначе он писал бы
        // в базу строку, которую кольцо тут же удаляет, а пользователь с
        // `persist = true` думал бы, что работа переживает перезапуск.
        let sink_failed = if self.persist && self.store.keep() > 0 {
            // Своё подключение к той же базе: буфер живёт дольше сессии.
            let store = self.store.reopen().map_err(|e| SessionError::Store {
                path: self.paths.database().display().to_string(),
                reason: e.to_string(),
            })?;
            let failed: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
            buffer.set_sink(
                path.clone(),
                Box::new(StoreSink {
                    store,
                    failed: Rc::clone(&failed),
                }),
            );
            Some(failed)
        } else {
            None
        };
        // Без приёмника ошибки писать некуда: ячейка всё равно нужна, чтобы
        // документ ждал одинаково в обоих случаях.
        let sink_failed = sink_failed.unwrap_or_default();
        Ok(Document {
            path,
            // Карта строится по тексту, а не едет дельтами: документ только что
            // открыт, истории правок ещё не было.
            lines: LineMap::build(&buffer.read()),
            buffer,
            changed_outside,
            sink_failed,
            snapshot_failed: None,
            interval: self.interval,
            last_snapshot: Duration::ZERO,
            clock: Box::new(SystemClock),
            stats: Stats::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// Отдельный каталог под каждый тест: базы не делятся.
    struct Temp {
        dir: PathBuf,
    }

    impl Temp {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("claxis-session-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }

        /// Файл документа рядом с базой.
        fn file(&self, name: &str) -> PathBuf {
            self.dir.join(name)
        }

        fn session(&self, keep: u8) -> Session {
            Session::open_at(self.dir.clone(), keep).unwrap()
        }

        /// Сессия с короткой историей: снапшот срабатывает быстро.
        fn short_session(&self, keep: u8, depth: u32) -> Session {
            Session::open_at_with(self.dir.clone(), keep, true, depth).unwrap()
        }

        /// Сессия, которая не пишет снапшоты на диск.
        fn no_persist_session(&self, keep: u8, depth: u32) -> Session {
            Session::open_at_with(self.dir.clone(), keep, false, depth).unwrap()
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Набрать текст и снять снапшот, дождавшись интервала на поддельных часах.
    fn type_until_snapshot(session: &Session, file: &Path, chunk: &str) -> usize {
        let mut doc = session.open_file(file).unwrap();
        // Пока часы стоят, снапшот не снимется ни при какой правке.
        doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
        let mut i = 0usize;
        while doc.buffer().snapshots_taken() == 0 && i < 40 {
            let pos = doc.buffer().len();
            doc.insert(pos, chunk.as_bytes()).unwrap();
            doc.snapshot_if_due();
            i += 1;
        }
        // Пять минут прошли — снимок снимает всё, что набрано.
        doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(600))));
        doc.snapshot_if_due();
        assert_eq!(doc.buffer().snapshots_taken(), 1, "снапшот не сработал");
        i
    }

    /// Документ с настоящей сессией и настоящим приёмником.
    fn doc_in(dir: &Path) -> Document {
        let file = dir.join("a.rs");
        std::fs::write(&file, "abc\n").unwrap();
        Session::open_at_with(dir, 3, true, 64)
            .unwrap()
            .open_file(&file)
            .unwrap()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("claxis-session-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_snapshot_that_failed_to_write_is_reported() {
        // Приёмник снапшотов не может вернуть ошибку: у него нет вызывающего.
        // Молчать об этом нельзя — человек закрыл бы редактор, решив, что работа
        // сохранена, и потерял бы её.
        let dir = temp_dir("snapshot-failed");
        let mut doc = doc_in(&dir);

        // Ошибка приходит от приёмника в общую ячейку — так же, как при
        // настоящей поломке диска.
        *doc.sink_failed.borrow_mut() = Some("диск кончился".to_string());

        doc.snapshot();
        let reason = doc
            .snapshot_error()
            .expect("ошибка записи снапшота обязана дойти до документа");
        assert_eq!(reason, "диск кончился");
        // Второй раз то же не повторяется: показали один раз и хватит.
        assert!(
            doc.snapshot_error().is_none(),
            "одна ошибка — одно сообщение"
        );
    }

    #[test]
    fn a_snapshot_written_without_trouble_says_nothing() {
        let dir = temp_dir("snapshot-ok");
        let mut doc = doc_in(&dir);
        doc.snapshot();
        assert!(
            doc.snapshot_error().is_none(),
            "снапшот лёг, жаловаться не на что"
        );
    }

    #[test]
    fn a_document_without_a_sink_has_nothing_to_report() {
        // Хранилище выключено: писать некуда и жаловаться не на что.
        let dir = temp_dir("no-sink");
        let file = dir.join("b.rs");
        std::fs::write(&file, "abc\n").unwrap();
        let session = Session::open_at_with(&dir, 3, false, 64).unwrap();
        let mut doc = session.open_file(&file).unwrap();
        doc.snapshot();
        assert!(doc.snapshot_error().is_none());
    }

    // ── Связка буфера и хранилища ──────────────────────────────────────────────

    #[test]
    fn snapshot_reaches_disk() {
        // Правки доходят до снапшота на диске — ради этого всё и затевалось.
        let tmp = Temp::new("to-disk");
        let file = tmp.file("a.rs");
        let session = tmp.session(3);

        let typed = type_until_snapshot(&session, &file, "abc");
        assert!(typed > 0);

        let snaps = session.store().list(&file).unwrap();
        assert!(!snaps.is_empty(), "снапшот должен оказаться в базе");
        assert_eq!(
            snaps[0].text,
            "abc".repeat(typed).into_bytes(),
            "в базе лежит всё, что набрано к моменту снапшота"
        );
    }

    #[test]
    fn snapshots_survive_restart() {
        // Главное: второй запуск редактора видит снятое ранее.
        let tmp = Temp::new("restart");
        let file = tmp.file("a.rs");
        let typed = {
            let session = tmp.session(3);
            type_until_snapshot(&session, &file, "xy")
        };
        {
            let session = tmp.session(3);
            let snaps = session.store().list(&file).unwrap();
            assert!(!snaps.is_empty(), "после перезапуска снапшоты должны быть");
            assert_eq!(snaps[0].text.len(), typed * 2);
        }
    }

    #[test]
    fn file_text_is_the_source_of_truth() {
        // Снапшот не заменяет файл: документ читается с диска, а не из кеша.
        let tmp = Temp::new("file-truth");
        let file = tmp.file("a.rs");
        std::fs::write(&file, "содержимое файла".as_bytes()).unwrap();

        let session = tmp.session(3);
        let doc = session.open_file(&file).unwrap();
        assert_eq!(doc.text(), "содержимое файла".as_bytes());
        assert_eq!(doc.path(), file.as_path());
    }

    #[test]
    fn missing_file_opens_as_empty() {
        // Файла нет — это новый документ, а не ошибка.
        let tmp = Temp::new("missing");
        let session = tmp.session(3);
        let doc = session.open_file(tmp.file("нет-такого.rs")).unwrap();
        assert!(doc.text().is_empty());
    }

    #[test]
    fn two_documents_write_separate_snapshots() {
        // Два документа в одной сессии не мешают друг другу.
        let tmp = Temp::new("two-docs");
        let a = tmp.file("a.rs");
        let b = tmp.file("b.rs");
        let session = tmp.session(3);

        type_until_snapshot(&session, &a, "a");
        type_until_snapshot(&session, &b, "b");

        let sa = session.store().latest(&a).unwrap().expect("снапшот a");
        let sb = session.store().latest(&b).unwrap().expect("снапшот b");
        assert!(sa.text.starts_with(b"a"));
        assert!(sb.text.starts_with(b"b"));
    }

    #[test]
    fn keep_limit_applies_through_session() {
        // Настройка конфига доходит до кольца в базе.
        let tmp = Temp::new("keep");
        let file = tmp.file("a.rs");
        // Короткая история: снапшот срабатывает каждые несколько записей.
        let session = tmp.short_session(2, 8);

        let mut doc = session.open_file(&file).unwrap();
        // Часы стоят: ни одна из сорока вставок не успевает за интервал.
        doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
        for _ in 0..40 {
            let end = doc.buffer().len();
            doc.insert(end, b"z").unwrap();
            doc.snapshot_if_due();
        }
        assert_eq!(
            doc.buffer().snapshots_taken(),
            0,
            "за сорок вставок подряд снапшот не заслужили"
        );
        drop(doc);

        let count = session.store().count(&file).unwrap();
        assert!(
            count <= 2,
            "в базе должно быть не больше keep снапшотов, а их {count}"
        );
    }

    #[test]
    fn unreadable_file_reports_error() {
        // Каталог на месте файла — это не «нет файла», а ошибка чтения.
        let tmp = Temp::new("unreadable");
        let dir = tmp.dir.join("папка");
        std::fs::create_dir_all(&dir).unwrap();
        let session = tmp.session(3);
        let err = session
            .open_file(&dir)
            .expect_err("чтение каталога должно падать");
        match err {
            SessionError::ReadFile { path, .. } => assert!(path.contains("папка")),
            other => panic!("не та ошибка: {other}"),
        }
    }

    #[test]
    fn buffer_and_store_do_not_know_each_other() {
        // Стык сделан в роутере: крейты не зависят друг от друга.
        // Проверяем, что документ работает и без приёмника вообще.
        let mut doc = crate::api::rope::Document::new("начало".as_bytes());
        for _ in 0..9000 {
            doc.insert(doc.len(), b"x").unwrap();
        }
        assert_eq!(
            doc.snapshots_taken(),
            0,
            "документ сам снимает только по прямой просьбе"
        );
        doc.snapshot();
        assert!(doc.snapshots_taken() > 0, "снапшот берётся и без приёмника");
        assert_eq!(doc.read().len(), 9000 + "начало".len());
    }

    #[test]
    fn history_depth_reaches_the_buffer() {
        // Глубина истории из конфига доходит до буфера.
        let tmp = Temp::new("depth");
        let file = tmp.file("a.rs");
        let session = tmp.short_session(3, 4);
        assert_eq!(session.history_depth(), 4);
        let doc = session.open_file(&file).unwrap();
        assert_eq!(doc.buffer().history_depth(), 4);
    }

    #[test]
    fn bad_history_depth_reports_error() {
        // Недопустимая глубина — ошибка, а не паника.
        let tmp = Temp::new("bad-depth");
        let file = tmp.file("a.rs");
        let session = tmp.short_session(3, 0);
        let err = session
            .open_file(&file)
            .expect_err("нулевая глубина недопустима");
        assert!(matches!(err, SessionError::Buffer(_)), "получено: {err}");
    }

    #[test]
    fn session_opens_in_standard_cache_path() {
        // Путь по умолчанию — кеш ОС, а не рабочий каталог.
        let paths = crate::api::store::StorePaths::from_env();
        let dir = paths.dir.display().to_string();
        assert!(
            dir.ends_with("claxis"),
            "путь должен кончаться на claxis: {dir}"
        );
        assert!(
            dir.contains("cache") || dir.contains("Cache"),
            "это должен быть кеш: {dir}"
        );
    }

    #[test]
    fn file_too_large_reports_bytes_and_limit() {
        // Файл больше u32::MAX не помещается в буфер.
        let err = SessionError::FileTooLarge {
            bytes: 5_000_000_000,
        };
        let text = err.text();

        // Точный размер в байтах.
        assert!(
            text.contains("5 000 000 000 bytes"),
            "в сообщении должен быть точный размер в байтах: {text}"
        );
        // Размер файла в десятичных гигабайтах: 5 000 000 000 / 10^9 = 5.0.
        assert!(
            text.contains("(5.0 GB)"),
            "размер файла должен быть в десятичных ГБ: {text}"
        );
        // Предел — точные байты: u32::MAX = 4 294 967 295.
        assert!(
            text.contains("\nthe maximum is 4 294 967 295 bytes"),
            "предел должен быть в точных байтах: {text}"
        );
        // Гигабайты приложены для удобства: 4.29 ГБ, а не 4.0.
        assert!(
            text.contains("(4.3 GB)"),
            "предел в ГБ должен быть 4.3, а не 4.0: {text}"
        );
    }

    #[test]
    fn gigabytes_are_decimal_not_binary() {
        // Гигабайт считается в 10^9. Если считать в 2^30, то получится 4.0 и
        // пользователю покажут неверный предел.
        let limit_gb = u32::MAX as f64 / 1_000_000_000.0;
        assert!(
            (limit_gb - 4.294967295).abs() < 1e-9,
            "предел должен быть 4.294967295 ГБ, получено {limit_gb}"
        );
        assert!(
            (u32::MAX as f64 / 1_073_741_824.0 - 4.0).abs() < 1e-9,
            "деление на 2^30 даёт 4.0 — это гибибайты, а не гигабайты"
        );
    }

    #[test]
    fn big_numbers_are_split_into_readable_parts() {
        use crate::api::text::group_digits;

        // Без разделителей длинное число не читается.
        assert_eq!(group_digits(4_294_967_295), "4 294 967 295");
        assert_eq!(group_digits(1_000), "1 000");
        assert_eq!(group_digits(1_234_567_890), "1 234 567 890");
        // Числа короче трёх разрядов не трогаем.
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(100), "100");
        assert_eq!(group_digits(0), "0");
    }

    #[test]
    fn persist_false_keeps_snapshots_in_memory_only() {
        // Настройка persist обязана что-то значить: при false снапшоты не пишутся.
        let tmp = Temp::new("no-persist");
        let file = tmp.file("a.rs");
        let session = tmp.no_persist_session(3, 8);

        let mut doc = session.open_file(&file).unwrap();
        doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
        for _ in 0..40 {
            let end = doc.buffer().len();
            doc.insert(end, b"z").unwrap();
        }
        doc.snapshot();
        // Снапшот взят, текст в буфере есть.
        assert_eq!(doc.buffer().snapshots_taken(), 1, "снапшот взят");
        assert_eq!(doc.buffer().len(), 40, "правки на месте");
        drop(doc);

        // Но на диске пусто.
        assert_eq!(
            session.store().count(&file).unwrap(),
            0,
            "при persist = false на диск не пишется ничего"
        );
    }

    #[test]
    fn defaults_come_from_crate_settings() {
        // Сессия по умолчанию берёт значения из настроек крейтов, а не из
        // зашитых констант.
        let tmp = Temp::new("defaults");
        let session = Session::open_at(tmp.dir.clone(), crate::api::store::KEEP).unwrap();
        assert_eq!(
            session.history_depth(),
            crate::api::rope::defaults::buffer::HISTORY_DEPTH.value
        );
        assert_eq!(session.persist(), crate::api::store::PERSIST);
    }

    #[test]
    fn history_survives_reopening_the_document() {
        // Главное: правки и снапшот переживают перезапуск. Текст верный, но без
        // истории отменять нечего — половина работы пользователя исчезает.
        let tmp = Temp::new("history-survives");
        std::fs::create_dir_all(&tmp.dir).unwrap();
        let file = tmp.dir.join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();

        let session = Session::open_at_with(tmp.dir.clone(), 3, true, 64).unwrap();

        {
            let mut doc = session.open_file(&file).unwrap();
            doc.insert(3, b"XY").unwrap();
            doc.delete(0, 1).unwrap();
            assert_eq!(doc.text(), b"bcXY\n");
            // Снапшот уносит историю на диск.
            doc.snapshot();
        }

        // Новый процесс читает тот же файл.
        let session2 = Session::open_at_with(tmp.dir.clone(), 3, true, 64).unwrap();
        let mut doc = session2.open_file(&file).unwrap();

        // Текст на месте.
        assert_eq!(doc.text(), b"bcXY\n");
        // И история на месте: отмена работает после перезапуска.
        doc.undo()
            .unwrap()
            .expect("отмена доступна после перезапуска");
        assert_eq!(doc.text(), b"abcXY\n");
    }

    #[test]
    fn a_file_changed_outside_is_not_overwritten_by_the_snapshot() {
        // Файл правили не из редактора. Снапшот построен на прежней версии, и
        // применять его нельзя: правка извне стала бы невидимой и уехала бы под
        // следующим сохранением.
        let tmp = Temp::new("changed-outside");
        let file = tmp.dir.join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();

        let session = Session::open_at_with(tmp.dir.clone(), 3, true, 64).unwrap();
        {
            let mut doc = session.open_file(&file).unwrap();
            doc.insert(3, b"XY").unwrap();
            doc.snapshot();
        }

        // Правка снаружи: и содержимое, и длина.
        std::fs::write(&file, "чужая правка\n").unwrap();

        let session2 = Session::open_at_with(tmp.dir.clone(), 3, true, 64).unwrap();
        let doc = session2.open_file(&file).unwrap();
        assert_eq!(
            doc.text(),
            "чужая правка\n".as_bytes(),
            "открыт файл как есть"
        );
        assert!(doc.changed_outside(), "об изменении сообщаем");
    }

    #[test]
    fn keep_zero_means_memory_only_and_writes_nothing() {
        // Ноль в кольце — это «только память». Приёмника быть не должно: иначе
        // он писал бы в базу строку, которую кольцо тут же удаляет, а
        // пользователь с `persist = true` думал бы, что работа переживает
        // перезапуск.
        let tmp = Temp::new("keep-zero");
        let file = tmp.dir.join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();

        let session = Session::open_at_with(tmp.dir.clone(), 0, true, 64).unwrap();
        {
            let mut doc = session.open_file(&file).unwrap();
            doc.insert(3, b"XY").unwrap();
            doc.snapshot();
        }

        let store = session.store().reopen().unwrap();
        assert!(
            store.latest(&file).unwrap().is_none(),
            "в базу не должно уйти ничего"
        );
    }

    #[test]
    fn an_untouched_file_still_gets_its_snapshot() {
        // Обратная сторона проверки: файл не трогали, снапшот применяется.
        let tmp = Temp::new("untouched");
        let file = tmp.dir.join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();

        let session = Session::open_at_with(tmp.dir.clone(), 3, true, 64).unwrap();
        {
            let mut doc = session.open_file(&file).unwrap();
            doc.insert(3, b"XY").unwrap();
            doc.snapshot();
        }

        let session2 = Session::open_at_with(tmp.dir.clone(), 3, true, 64).unwrap();
        let doc = session2.open_file(&file).unwrap();
        assert_eq!(doc.text(), b"abcXY\n", "текст из снапшота");
        assert!(!doc.changed_outside(), "файл не меняли, ложной тревоги нет");
    }

    #[test]
    fn a_foreign_history_blob_does_not_break_the_file() {
        // В базе может лежать блоб прежнего буфера: он писал в ту же колонку
        // свой формат. Такой блоб историю не восстановит, но файл обязан
        // открыться, а текст — оказаться на месте.
        let tmp = Temp::new("foreign-blob");
        let file = tmp.dir.join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();

        let session = Session::open_at_with(tmp.dir.clone(), 3, true, 64).unwrap();
        {
            let mut store = session.store().reopen().unwrap();
            let mut old = b"BUFH".to_vec();
            old.extend_from_slice(&[0u8; 40]);
            store.put(&file, "abc\n".as_bytes(), &old).unwrap();
        }

        let session2 = Session::open_at_with(tmp.dir.clone(), 3, true, 64).unwrap();
        let mut doc = session2.open_file(&file).unwrap();
        assert_eq!(doc.text(), b"abc\n", "текст из снапшота на месте");
        // Истории нет — отменять нечего, и это не ошибка.
        assert!(doc.undo().unwrap().is_none());
        doc.insert(3, b"Z").unwrap();
        assert_eq!(doc.text(), b"abcZ\n", "документ рабочий");
    }

    #[test]
    fn a_rope_snapshot_is_written_in_the_rope_format() {
        // Что уходит в базу, обязан быть форматом rope: с магией и версией.
        // Иначе следующий запуск не поймёт, что там лежит.
        let tmp = Temp::new("rope-format");
        let file = tmp.dir.join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();

        let session = Session::open_at_with(tmp.dir.clone(), 3, true, 64).unwrap();
        {
            let mut doc = session.open_file(&file).unwrap();
            doc.insert(3, b"XY").unwrap();
            doc.snapshot();
        }

        let store = session.store().reopen().unwrap();
        let snapshot = store.latest(&file).unwrap().expect("снапшот записан");
        assert_eq!(&snapshot.history[..4], b"CLRP", "магия формата rope");
        assert!(snapshot.history.len() > 5, "в блобе есть версия");

        // И разбирается тем же кодом, без потерь.
        let restored = crate::api::rope::History::decode(&snapshot.history)
            .expect("блоб из базы — наш")
            .restore()
            .expect("документ собирается");
        assert_eq!(restored.read(), b"abcXY\n");
    }

    #[test]
    fn a_snapshot_is_taken_by_the_clock_and_not_by_the_edit_count() {
        // Снапшот снимает время. Правок может быть сколько угодно, и лимит на
        // них не влияет: интервал честно отсчитывается часами.
        let tmp = Temp::new("snapshot-by-time");
        let file = tmp.dir.join("doc.txt");
        std::fs::write(&file, "x\n").unwrap();

        let mut session = Session::open_at_with(tmp.dir.clone(), 3, true, 64).unwrap();
        session.set_snapshot_interval(Duration::from_secs(300));

        // Часы стоят: сколько правок ни сделай, пяти минут не прошло.
        let mut doc = session.open_file(&file).unwrap();
        doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
        for _ in 0..500 {
            let at = doc.buffer().len() - 1;
            doc.insert(at, b"y").unwrap();
            doc.snapshot_if_due();
        }
        assert_eq!(doc.text().len(), 502, "правки на месте");
        assert_eq!(doc.buffer().snapshots_taken(), 0, "время не шло");

        // Часы пошли: пять минут прошло, и первая же проверка снимает снапшот.
        doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(299))));
        doc.snapshot_if_due();
        assert_eq!(doc.buffer().snapshots_taken(), 0, "до интервала рано");

        doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(300))));
        doc.snapshot_if_due();
        assert_eq!(
            doc.buffer().snapshots_taken(),
            1,
            "интервал вышел, снимок есть"
        );

        // И отсчёт начинается заново: следующий снимок ещё не через пять минут.
        doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(420))));
        doc.snapshot_if_due();
        assert_eq!(doc.buffer().snapshots_taken(), 1, "отсчёт пошёл заново");
    }

    #[test]
    fn the_interval_comes_from_the_settings() {
        // Интервал из конфига доходит до сессии и до документа.
        let tmp = Temp::new("interval-setting");
        let file = tmp.file("a.rs");
        let mut session = tmp.session(3);
        session.set_snapshot_interval(Duration::from_secs(60));
        assert_eq!(session.snapshot_interval(), Duration::from_secs(60));

        let doc = session.open_file(&file).unwrap();
        assert_eq!(doc.snapshot_interval(), Duration::from_secs(60));
    }

    #[test]
    fn a_zero_interval_snapshots_on_every_edit() {
        // Нулевой интервал — это «на каждой правке». В конфиге так не выставляют,
        // но без него не проверить, что счётчик не врёт.
        let tmp = Temp::new("zero-interval");
        let file = tmp.file("a.rs");
        let mut session = tmp.session(3);
        session.set_snapshot_interval(Duration::ZERO);

        let mut doc = session.open_file(&file).unwrap();
        for i in 0..5 {
            let at = doc.buffer().len();
            doc.insert(at, b"x").unwrap();
            doc.snapshot_if_due();
            assert_eq!(i + 1, doc.buffer().snapshots_taken() as u32);
        }
    }

    #[test]
    fn a_missing_file_opens_as_an_empty_document() {
        // Файла нет — это не ошибка, а новый документ. Снапшот такого должен
        // разобраться обратно.
        let tmp = Temp::new("missing-file-snapshot");
        let file = tmp.file("нет-такого.rs");
        let session = tmp.session(3);

        let mut doc = session.open_file(&file).unwrap();
        assert_eq!(doc.text(), Vec::<u8>::new(), "пустой документ");
        doc.insert(0, "новый текст".as_bytes()).unwrap();
        doc.snapshot();
        assert_eq!(doc.text(), "новый текст".as_bytes());

        // С диска читается то, что осталось, и история не ломает его.
        let session2 = tmp.session(3);
        let mut back = session2.open_file(&file).unwrap();
        assert_eq!(back.text(), "новый текст".as_bytes());
        back.undo().unwrap().expect("история из снапшота живая");
        assert_eq!(back.text(), Vec::<u8>::new());
    }

    // ── Статистика работы ──────────────────────────────────────────────────────

    #[test]
    fn the_statistics_counts_what_the_user_did() {
        // Счётчик живёт в документе сессии: он видит правки, документ — нет.
        let tmp = Temp::new("stats");
        let file = tmp.file("a.rs");
        let session = tmp.session(3);
        let mut doc = session.open_file(&file).unwrap();

        doc.insert(0, "абв".as_bytes()).unwrap();
        doc.insert(3, "г".as_bytes()).unwrap();
        doc.delete(0, 1).unwrap();
        // «абв» — три буквы, а не шесть байт.
        assert_eq!(doc.stats().input_chars, 4);
        assert_eq!(doc.stats().deleted_chars, 1);
        assert_eq!(doc.stats().edits, 3);
    }

    #[test]
    fn the_statistics_count_only_the_time_a_key_was_held() {
        // Секунды между нажатиями — это размышление, а не печать.
        let tmp = Temp::new("presses");
        let file = tmp.file("a.rs");
        let session = tmp.session(3);
        let mut doc = session.open_file(&file).unwrap();

        doc.note_press();
        // Между нажатиями прошло двадцать секунд: они сюда не попадают.
        doc.note_press();
        doc.note_held(Duration::from_millis(70));
        doc.note_held(Duration::from_millis(130));
        // Третье нажатие так и не отпустили: время оно не добавит.
        doc.note_press();

        assert_eq!(doc.stats().presses, 3, "нажатий три");
        assert_eq!(
            doc.stats().pressed,
            Duration::from_millis(200),
            "время заняли сами нажатия"
        );
    }

    #[test]
    fn replace_counts_both_sides() {
        // Замена — это удаление и вставка, и статистика должна видеть обе.
        let tmp = Temp::new("stats-replace");
        let file = tmp.file("a.rs");
        let session = tmp.session(3);
        let mut doc = session.open_file(&file).unwrap();

        doc.insert(0, "строка".as_bytes()).unwrap();
        assert_eq!(doc.stats().edits, 1);

        // Позиции и длины — в байтах, а не в буквах: «строка» это 12 байт.
        let len = "строка".len() as u32;
        doc.replace(0, len, "другая".as_bytes()).unwrap();
        let stats = doc.stats();
        assert_eq!(stats.edits, 2, "замена — одна правка");
        assert_eq!(stats.input_chars, 6 + 6, "шесть букв, потом шесть");
        assert_eq!(stats.deleted_chars, u64::from(len));
        assert_eq!(doc.text(), "другая".as_bytes());
    }

    #[test]
    fn statistics_survive_a_snapshot() {
        // Снапшот чинит документ, но не обнуляет счётчики: статистика за
        // сеанс считается с открытия файла, а не с последнего снимка.
        let tmp = Temp::new("stats-snapshot");
        let file = tmp.file("a.rs");
        let session = tmp.session(3);
        let mut doc = session.open_file(&file).unwrap();

        doc.insert(0, "до".as_bytes()).unwrap();
        doc.note_press();
        doc.note_held(Duration::from_millis(50));
        doc.snapshot();
        assert_eq!(doc.stats().input_chars, 2, "счётчик не сбросился");
        assert_eq!(doc.stats().presses, 1);

        doc.insert(0, "после".as_bytes()).unwrap();
        assert_eq!(doc.stats().input_chars, 7, "счётчик считает и дальше");
        assert_eq!(doc.stats().edits, 2);
    }

    #[test]
    fn a_document_survives_a_thousand_tiny_edits() {
        // Много мелких правок подряд: дерево перестраивается на каждой, и ни
        // одна из них не должна его сломать.
        let tmp = Temp::new("tiny-edits");
        let file = tmp.file("a.rs");
        let session = tmp.session(3);
        let mut doc = session.open_file(&file).unwrap();

        for i in 0..1000u32 {
            doc.insert(0, "•".as_bytes()).unwrap();
            // Булет — три байта, как и любая буква кириллицы.
            assert_eq!(doc.buffer().len(), (i + 1) * 3);
        }
        assert_eq!(doc.text().len(), 3000);
        assert_eq!(doc.buffer().read_range(0, 0), Vec::<u8>::new());
    }

    // ── Карта строк едет вместе с документом ──────────────────────────────────

    #[test]
    fn the_line_map_stays_with_the_document() {
        // Карта строк обязана ехать вместе с текстом на каждой правке, отмене и
        // повторе. Эталон — обычная строка с историей правок: любое расхождение
        // означает, что карта разъехалась.
        let tmp = Temp::new("line-map");
        let file = tmp.file("a.rs");
        let session = tmp.session(3);
        let mut doc = session.open_file(&file).unwrap();

        let mut text = String::new();
        let mut history: Vec<Op> = Vec::new();
        let mut seed = 0x2545F4914F6CDD1Du64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n.max(1) as u64) as usize
        };

        for _ in 0..600 {
            // Что именно сделали на этом шаге: без этого расхождение
            // бесполезно.
            let done: String;
            match next(5) {
                0 if !text.is_empty() => {
                    let at = next(text.len());
                    let len = 1 + next((text.len() - at).min(20));
                    let removed = text[at..at + len].to_string();
                    doc.delete(at as u32, len as u32).unwrap();
                    text.replace_range(at..at + len, "");
                    history.push(Op::Delete { at, removed });
                    done = format!("удалить {at}..{}", at + len);
                }
                1 => {
                    let at = next(text.len() + 1);
                    let typed = if next(2) == 0 { "\n" } else { "ab" };
                    doc.insert(at as u32, typed.as_bytes()).unwrap();
                    text.insert_str(at, typed);
                    history.push(Op::Insert {
                        at,
                        len: typed.len(),
                    });
                    done = format!("вставить {typed:?} на {at}");
                }
                2 => {
                    let at = next(text.len() + 1);
                    let len = next((text.len() - at).min(10));
                    let removed = text[at..at + len].to_string();
                    let typed = "xy\n";
                    doc.replace(at as u32, len as u32, typed.as_bytes())
                        .unwrap();
                    text.replace_range(at..at + len, typed);
                    history.push(Op::Replace { at, removed });
                    done = format!("заменить {at}..{} на {typed:?}", at + len);
                }
                3 => {
                    // Снапшот текста не меняет, и карта строк после него обязана
                    // остаться в силе. Раньше её пересобирали зря. Зато история
                    // правок обнуляется — как и в эталоне.
                    doc.snapshot();
                    history.clear();
                    done = "снапшот".to_string();
                }
                _ => {
                    let Some(op) = history.pop() else { continue };
                    doc.undo().unwrap().expect("отменять есть что");
                    done = format!("отменить {op:?}");
                    match op {
                        Op::Insert { at, len } => text.replace_range(at..at + len, ""),
                        Op::Delete { at, removed, .. } => text.insert_str(at, &removed),
                        // Отмена замены убирает вставленное и возвращает
                        // вырезанное.
                        Op::Replace { at, removed, .. } => {
                            text.replace_range(at..at + 3, "");
                            text.insert_str(at, &removed);
                        }
                    }
                }
            }

            let fresh = crate::router::LineMap::build(text.as_bytes());
            assert_eq!(
                doc.lines().breaks(),
                fresh.breaks(),
                "карта строк разошлась с текстом: {done}, текст {:?}",
                text
            );
            assert_eq!(doc.line_count(), fresh.line_count());
            assert_eq!(
                doc.text(),
                text.as_bytes(),
                "текст разошёлся: {done}, было {:?}",
                text
            );
        }
    }

    #[test]
    fn a_line_map_survives_undo_and_replace() {
        // Отмена вставки убирает её переводы строк, отмена удаления и замены
        // возвращает их на место.
        let tmp = Temp::new("lines-undo");
        let file = tmp.file("a.rs");
        let session = tmp.session(3);
        let mut doc = session.open_file(&file).unwrap();

        doc.insert(0, b"one\ntwo\n").unwrap();
        assert_eq!(doc.lines().breaks(), [3, 7]);
        doc.undo().unwrap();
        assert_eq!(doc.lines().breaks(), []);

        doc.replace(0, 0, b"a\nb").unwrap();
        assert_eq!(doc.lines().breaks(), [1]);
        doc.undo().unwrap();
        assert_eq!(doc.lines().breaks(), []);
    }

    /// Правка в эталоне: сколько байт затронуто и что было там до неё.
    #[derive(Debug)]
    enum Op {
        Insert { at: usize, len: usize },
        Delete { at: usize, removed: String },
        Replace { at: usize, removed: String },
    }
}

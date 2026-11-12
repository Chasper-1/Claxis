//! Открытый документ: текст, карта строк, статистика и снимки.
//!
//! Документ живёт в сессии, но не в ней: снапшот — это решение сессии, у неё
//! есть и документ, и хранилище. Сам документ про часы не знает и снимает
//! снапшот только по прямой просьбе.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use crate::api::buffer::{Document as Rope, Error as RopeError};
use crate::api::core::LineMap;
use crate::router::clock::Clock;
use crate::router::lines::Lines;
use crate::router::stats::{self, Stats};

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
    lines: Lines,
    /// Файл на диске разошёлся с тем, на котором строился снапшот.
    ///
    /// Снапшот тогда не применяется: иначе правка, сделанная снаружи, была бы
    /// не видна и уехала бы под следующим сохранением. Пользователю об этом
    /// говорит вызывающий.
    changed_outside: bool,
    /// Якорь «база ↔ диск» пропал, а правки в базе остались.
    ///
    /// Файл мог измениться, пока якоря не было, и проверять нечем. Новый
    /// якорь на месте правок не выдумывается: сохранение такого файла и
    /// дальше будет отказываться писать, пока правки не разберут руками.
    anchor_lost: bool,
    /// Документ записан на диск в этом сеансе.
    ///
    /// По нему выход решает, чистить ли историю файла в базе: несохранённые
    /// правки чистке не подлежат — это ещё не отражённая в файле работа.
    saved: bool,
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
        self.lines.map()
    }

    /// Файл меняли снаружи, пока редактор был закрыт.
    ///
    /// Снапшот тогда не применялся: иначе правка извне была бы не видна и
    /// уехала бы под следующим сохранением. Документ открыт как есть, и об
    /// этом честно сказано.
    pub fn changed_outside(&self) -> bool {
        self.changed_outside
    }

    /// Якорь «база ↔ диск» пропал, а правки в базе остались.
    ///
    /// Файл могли поменять, пока сверять было нечем: правки сохраняются
    /// как были, а файл на запись не пойдёт, пока человек не разберёт
    /// содержимое руками.
    pub fn anchor_lost(&self) -> bool {
        self.anchor_lost
    }

    /// Документ записан на диск в этом сеансе.
    pub fn saved(&self) -> bool {
        self.saved
    }

    /// Отметить документ записанным. Делает сохранение после успешной записи.
    pub fn mark_saved(&mut self) {
        self.saved = true;
    }

    /// Номер строки для позиции, начиная с нуля.
    pub fn line_of(&self, pos: u32) -> u32 {
        self.lines.map().line_of(pos)
    }

    /// Сколько строк в документе.
    pub fn line_count(&self) -> u32 {
        self.lines.map().line_count()
    }

    /// Позиция, с которой начинается строка.
    pub fn offset_of_line(&self, line: u32) -> u32 {
        self.lines.map().offset_of_line(line, self.buffer.len())
    }

    /// Позиция конца строки: сам перевод строки.
    pub fn end_of_line(&self, line: u32) -> u32 {
        self.lines.map().end_of_line(line, self.buffer.len())
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
        self.snapshot_if_over_depth();
        Ok(())
    }

    /// Вырезать диапазон и посчитать удалённые символы.
    pub fn delete(&mut self, anchor: u32, len: u32) -> Result<(), RopeError> {
        self.buffer.delete(anchor, len)?;
        self.lines.delete(anchor, len);
        stats::note_deleted(&mut self.stats, len);
        self.snapshot_if_over_depth();
        Ok(())
    }

    /// Заменить диапазон: вставка и удаление одной операцией.
    pub fn replace(&mut self, anchor: u32, len: u32, data: &[u8]) -> Result<(), RopeError> {
        self.buffer.replace(anchor, len, data)?;
        self.lines.replace(anchor, len, data);
        stats::note_replace(&mut self.stats, data, len);
        self.snapshot_if_over_depth();
        Ok(())
    }

    /// Отменить правку. Карта строк едет вместе с текстом.
    pub fn undo(&mut self) -> Result<Option<u32>, RopeError> {
        let path = self.buffer.path();
        let Some(id) = self.buffer.undo()? else {
            return Ok(None);
        };
        // Запись узнаём по её месту в ветке: отмена убрала последнюю.
        if let Some(record) = path.get(path.len().wrapping_sub(1)) {
            let removed = self.buffer.removed_text(record).map(|text| text.to_vec());
            self.lines.undo(record, removed.as_deref());
        }
        Ok(Some(id))
    }

    /// Повторить отменённую правку. Карта строк едет вместе с текстом.
    pub fn redo(&mut self) -> Result<Option<u32>, RopeError> {
        let before = self.buffer.path().len();
        let Some(id) = self.buffer.redo()? else {
            return Ok(None);
        };
        // Повтор добавил одну запись в конец ветки.
        if let Some(record) = self.buffer.path().get(before) {
            let inserted = self.buffer.inserted_text(record).map(|text| text.to_vec());
            self.lines.redo(record, inserted.as_deref());
        }
        Ok(Some(id))
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

    /// Снять снапшот, если с прошлого прошло достаточно времени.
    ///
    /// Время, а не число правок: человек может править один и тот же абзац
    /// десять минут, и снапшот каждые пять минут отдаст память обратно. И
    /// наоборот — сто правок подряд за минуту снапшот не заслужили. Предел
    /// памяти — отдельный повод, и он проверяется прямо на правке.
    pub fn snapshot_if_due(&mut self) {
        // Ноль — это «снимков нет вообще», а не «на каждой правке». Иначе
        // человек, выключивший интервал, получил бы обратный смысл настройки
        // и вдвое больше работы на диске, чем просил.
        if self.interval.is_zero() {
            return;
        }
        if self.clock.elapsed().saturating_sub(self.last_snapshot) >= self.interval {
            self.snapshot();
        }
    }

    /// Снять снапшот и отсчитать время заново.
    pub fn snapshot(&mut self) {
        // Текст от этого не меняется, поэтому карта строк остаётся в силе.
        self.buffer.snapshot();
        self.take_sink_failure();
        self.last_snapshot = self.clock.elapsed();
    }

    /// Снять снапшот, если с прошлого прошло достаточно правок.
    ///
    /// Это второй повод для снимка, кроме времени: время сохраняет сессию, а
    /// глубина задаёт шаг между снимками. Время сдвигается только когда снимок
    /// правда снят, — иначе следующий плановый снимок отложился бы.
    fn snapshot_if_over_depth(&mut self) {
        let before = self.buffer.snapshots_taken();
        self.buffer.snapshot_if_over_depth();
        // Ошибку приёмника забираем в любом случае: снимок мог не лечь, и
        // тогда история осталась в памяти — об этом человек обязан узнать.
        self.take_sink_failure();
        if self.buffer.snapshots_taken() != before {
            self.last_snapshot = self.clock.elapsed();
        }
    }

    /// Забрать ошибку приёмника, если он не смог записать снимок.
    fn take_sink_failure(&mut self) {
        // Приёмник отдал текст и историю и ошибся мог там, где у него нет
        // вызывающего. Ошибку забираем здесь: она про работу пользователя, а не
        // про кеш, и молчать о ней нельзя.
        if let Some(reason) = self.sink_failed.borrow_mut().take() {
            self.snapshot_failed = Some(reason);
        }
    }

    /// Снапшот не лёг на диск. Текст с причиной, уже переведённый.
    ///
    /// Пока ошибка не забрана вызывающим, она здесь: следующий вызов вернёт её
    /// и очистит, чтобы одно и то же не сообщать дважды.
    pub fn snapshot_error(&mut self) -> Option<String> {
        self.snapshot_failed.take()
    }
}

/// Части, из которых собирается документ.
///
/// Сессия открывает документ и знает всё, что в нём должно быть; собирать
/// его изнутри неудобно — отсюда отдельная структура: поля документа
/// остаются закрытыми, а собирается он в одном месте.
pub(super) struct Parts {
    /// Путь к файлу.
    pub path: PathBuf,
    /// Текст с историей.
    pub buffer: Rope,
    /// Через сколько снимать следующий снапшот.
    pub interval: Duration,
    /// Откуда берётся время.
    pub clock: Box<dyn Clock>,
    /// Карта строк.
    pub lines: Lines,
    /// Файл на диске разошёлся с тем, на котором строился снапшот.
    pub changed_outside: bool,
    /// Якорь пропал, а правки в базе остались.
    pub anchor_lost: bool,
    /// Ячейка, куда приёмник кладёт ошибку записи снапшота.
    pub sink_failed: Rc<RefCell<Option<String>>>,
}

impl Document {
    /// Собрать документ из готовых частей.
    pub(super) fn from_parts(parts: Parts) -> Self {
        Self {
            path: parts.path,
            buffer: parts.buffer,
            interval: parts.interval,
            last_snapshot: Duration::ZERO,
            clock: parts.clock,
            stats: Stats::default(),
            lines: parts.lines,
            changed_outside: parts.changed_outside,
            anchor_lost: parts.anchor_lost,
            saved: false,
            sink_failed: parts.sink_failed,
            snapshot_failed: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Document;
    use crate::api::core::LineMap;
    use crate::router::clock::FixedClock;
    use crate::router::session::Session;
    use crate::router::session::testing::Temp;
    use std::path::Path;
    use std::time::Duration;

    /// Документ с настоящей сессией и настоящим приёмником.
    fn doc_in(dir: &Path) -> Document {
        let file = dir.join("a.rs");
        std::fs::write(&file, "abc\n").unwrap();
        Session::open_at_with(dir, 3, true, 64)
            .unwrap()
            .open_file(&file)
            .unwrap()
    }

    #[test]
    fn a_snapshot_that_failed_to_write_is_reported() {
        // Приёмник снапшотов не может вернуть ошибку: у него нет вызывающего.
        // Молчать об этом нельзя — человек закрыл бы редактор, решив, что работа
        // сохранена, и потерял бы её.
        let tmp = Temp::new("snapshot-failed");
        let mut doc = doc_in(tmp.dir());

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
        let tmp = Temp::new("snapshot-ok");
        let mut doc = doc_in(tmp.dir());
        doc.snapshot();
        assert!(
            doc.snapshot_error().is_none(),
            "снапшот лёг, жаловаться не на что"
        );
    }

    #[test]
    fn a_document_without_a_sink_has_nothing_to_report() {
        // Хранилище выключено: писать некуда и жаловаться не на что.
        let tmp = Temp::new("no-sink");
        let file = tmp.dir().join("b.rs");
        std::fs::write(&file, "abc\n").unwrap();
        let session = Session::open_at_with(tmp.dir(), 3, false, 64).unwrap();
        let mut doc = session.open_file(&file).unwrap();
        doc.snapshot();
        assert!(doc.snapshot_error().is_none());
    }

    #[test]
    fn undo_walks_back_through_all_edits() {
        // Снимки не обрезают память: человек отматывает всё, что наработал,
        // хотя на диск легли четыре снимка.
        let tmp = Temp::new("undo-through-ring");
        let file = tmp.file("a.rs");
        let session = tmp.short_session(4, 1_000);
        let mut doc = session.open_file(&file).unwrap();

        // Четыре снимка, между ними по одной правке.
        for round in 0..4u32 {
            doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
            doc.insert(doc.buffer().len(), b"x").unwrap();
            doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(
                300 * (round as u64 + 1),
            ))));
            doc.snapshot_if_due();
        }
        assert_eq!(session.store().count(&file).unwrap(), 4, "кольцо полное");

        // Теперь четыре правки назад — вся история ещё в памяти.
        for _ in 0..4 {
            doc.undo()
                .unwrap()
                .expect("откат по истории должен продолжаться");
        }
        assert!(doc.text().is_empty(), "откатилось не всё");
    }

    #[test]
    fn undo_stops_after_the_last_edit() {
        // Дальше основания истории идти некуда, и это должно быть понятно,
        // а не выглядеть как зависшая отмена.
        let tmp = Temp::new("undo-ring-end");
        let file = tmp.file("a.rs");
        let session = tmp.short_session(4, 1_000);
        let mut doc = session.open_file(&file).unwrap();

        doc.insert(doc.buffer().len(), b"x").unwrap();
        doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(300))));
        doc.snapshot_if_due();
        assert_eq!(doc.buffer().snapshots_taken(), 1, "снимок снят");
        assert_eq!(session.store().count(&file).unwrap(), 1, "и лежит в кольце");

        doc.undo().unwrap().expect("правка отменяется по истории");
        assert!(doc.text().is_empty(), "первая правка откатилась");
        assert!(
            doc.undo().unwrap().is_none(),
            "правка одна, дальше откатываться некуда"
        );
    }

    #[test]
    fn without_snapshots_undo_stops_at_the_base() {
        // `keep = 0` — снимки на диск не пишутся. Откат всё равно идёт по
        // памяти до основания: это честное «дальше некуда».
        let tmp = Temp::new("undo-no-ring");
        let file = tmp.file("a.rs");
        let session = tmp.no_persist_session(0, 1_000);
        let mut doc = session.open_file(&file).unwrap();

        doc.insert(doc.buffer().len(), b"x").unwrap();
        doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(300))));
        doc.snapshot_if_due();

        doc.undo()
            .unwrap()
            .expect("правка отменяется по памяти, кольца нет");
        assert!(doc.text().is_empty());
        assert!(
            doc.undo().unwrap().is_none(),
            "без кольца откат дальше не идёт"
        );
    }

    #[test]
    fn without_a_ring_the_memory_keeps_the_whole_undo_chain() {
        // `keep = 0` — это «снимков не делаем», а не «снимки делаем и роняем
        // историю». Без кольца история остаётся в памяти целиком.
        let tmp = Temp::new("no-ring-history");
        let file = tmp.file("a.rs");
        let session = tmp.session(0);
        let mut doc = session.open_file(&file).unwrap();
        doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));

        for _ in 0..5 {
            let at = doc.buffer().len();
            doc.insert(at, b"x").unwrap();
            doc.snapshot_if_due();
        }
        assert_eq!(
            session.store().count(&file).unwrap(),
            0,
            "на диск ничего не пишется"
        );
        assert_eq!(
            doc.buffer().path().len(),
            5,
            "история в памяти осталась целой"
        );

        for i in (0..5u32).rev() {
            doc.undo().unwrap().expect("откат по памяти");
            assert_eq!(doc.text(), "x".repeat(i as usize).into_bytes());
        }
        assert!(doc.undo().unwrap().is_none());
    }

    #[test]
    fn a_snapshot_does_not_stop_undo_while_it_is_still_written_to_disk() {
        // Обе способности сразу: история живёт в памяти целиком, и та же
        // история уезжает на диск. Отменять можно через любой снапшот.
        let tmp = Temp::new("undo-through-snapshot");
        let file = tmp.file("a.rs");
        let session = tmp.short_session(3, 1_000);
        let mut doc = session.open_file(&file).unwrap();

        // Часы стоят: правки идут, но пять минут ещё не прошло.
        doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
        for _ in 0..3 {
            let at = doc.buffer().len();
            doc.insert(at, b"x").unwrap();
            doc.snapshot_if_due();
        }
        assert_eq!(
            doc.buffer().snapshots_taken(),
            0,
            "за три правки подряд снапшот не заслужили"
        );

        // Пять минут прошло — снапшот снят, история на диске.
        doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(600))));
        doc.snapshot_if_due();
        let stored = session.store().count(&file).unwrap();
        assert!(stored > 0, "на диске должен быть хоть один снапшот");

        // Отмена идёт через снапшот насквозь: все три правки.
        for i in (0..3).rev() {
            doc.undo().unwrap().expect("отменять есть что");
            assert_eq!(doc.text(), b"x".repeat(i));
        }
        assert!(doc.undo().unwrap().is_none(), "дальше отменять нечего");
    }

    #[test]
    fn the_depth_takes_snapshots_without_cutting_history() {
        // Два повода для снимка: время сохраняет сессию, а глубина задаёт шаг
        // между снимками. История из памяти не уходит — иначе восстановить её
        // после перезапуска было бы не из чего.
        let tmp = Temp::new("depth-snapshot");
        let file = tmp.file("a.rs");
        let session = tmp.short_session(3, 4);

        let mut doc = session.open_file(&file).unwrap();
        doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
        for _ in 0..10 {
            let end = doc.buffer().len();
            doc.insert(end, b"x").unwrap();
            doc.snapshot_if_due();
        }

        assert_eq!(
            doc.buffer().snapshots_taken(),
            2,
            "глубина переросла дважды и каждый раз снимался снимок"
        );
        assert_eq!(
            doc.buffer().path().len(),
            10,
            "история целиком осталась в памяти"
        );
        assert_eq!(doc.text().len(), 10, "текст цел");

        for _ in 0..10 {
            doc.undo().unwrap().expect("откат идёт по памяти");
        }
        assert!(doc.undo().unwrap().is_none(), "дальше отменять нечего");
        assert!(doc.text().is_empty(), "дошли до пустого документа");
    }

    #[test]
    fn a_full_history_without_a_sink_keeps_the_edits() {
        // Без приёмника снимку некуда уйти. История при этом остаётся в
        // памяти: терять возможность отменить нельзя.
        let tmp = Temp::new("depth-no-sink");
        let file = tmp.file("a.rs");
        let session = tmp.no_persist_session(3, 4);

        let mut doc = session.open_file(&file).unwrap();
        for _ in 0..10 {
            let end = doc.buffer().len();
            doc.insert(end, b"x").unwrap();
            doc.snapshot_if_due();
        }
        assert_eq!(doc.buffer().snapshots_taken(), 0, "снимку некуда уйти");
        assert_eq!(doc.buffer().path().len(), 10, "история осталась целой");
        assert_eq!(session.store().count(&file).unwrap(), 0, "и на диске пусто");
    }

    #[test]
    fn a_snapshot_is_taken_by_the_clock_when_the_history_has_room() {
        // Снапшот снимает время, если глубина истории ещё не выбрана: правок
        // может быть много, и интервал честно отсчитывается часами.
        let tmp = Temp::new("snapshot-by-time");
        let file = tmp.file("doc.txt");
        std::fs::write(&file, "x\n").unwrap();

        let mut session = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 10_000).unwrap();
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
    fn a_zero_interval_means_no_snapshots_at_all() {
        // Ноль — это «снимков нет вообще», а не «на каждой правке». Иначе
        // человек, выключивший интервал, получил бы обратный смысл настройки и
        // вдвое больше работы на диске, чем просил.
        let tmp = Temp::new("zero-interval");
        let file = tmp.file("a.rs");
        let mut session = tmp.session(3);
        session.set_snapshot_interval(Duration::ZERO);

        let mut doc = session.open_file(&file).unwrap();
        for _ in 0..5 {
            let at = doc.buffer().len();
            doc.insert(at, b"x").unwrap();
            doc.snapshot_if_due();
        }
        assert_eq!(
            doc.buffer().snapshots_taken(),
            0,
            "при нулевом интервале снимков быть не должно"
        );
        assert_eq!(session.store().count(&file).unwrap(), 0, "и на диске тоже");
    }

    #[test]
    fn a_one_second_interval_snapshots_on_every_edit_past_it() {
        // Обратная сторона: интервал в секунду — это снимок раз в секунду, а не
        // раз в правку. Часы двигаем сами.
        let tmp = Temp::new("one-second");
        let file = tmp.file("a.rs");
        let mut session = tmp.session(3);
        session.set_snapshot_interval(Duration::from_secs(1));

        let mut doc = session.open_file(&file).unwrap();
        doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
        for _ in 0..3 {
            let at = doc.buffer().len();
            doc.insert(at, b"x").unwrap();
            doc.snapshot_if_due();
            assert_eq!(
                doc.buffer().snapshots_taken(),
                0,
                "за секунду снимок не заслужен"
            );
        }
        doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(1))));
        doc.snapshot_if_due();
        assert_eq!(doc.buffer().snapshots_taken(), 1, "секунда прошла");
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

    #[test]
    fn an_untouched_document_has_no_snapshot_to_restore_from() {
        // Обратная сторона: файл не трогали, но снапшот есть. Он и должен
        // применяться — иначе правка пропала бы.
        let tmp = Temp::new("untouched-snapshot");
        let file = tmp.file("a.rs");
        let session = tmp.short_session(3, 1_000);
        let mut doc = session.open_file(&file).unwrap();
        doc.insert(0, b"abc").unwrap();
        doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(600))));
        doc.snapshot_if_due();
        assert!(doc.snapshot_error().is_none());

        let session2 = tmp.short_session(3, 1_000);
        let mut back = session2.open_file(&file).unwrap();
        assert_eq!(back.text(), b"abc");
        back.undo().unwrap().expect("история из снапшота жива");
        assert!(back.text().is_empty());
    }

    #[test]
    fn buffer_and_store_do_not_know_each_other() {
        // Стык сделан в роутере: крейты не зависят друг от друга.
        // Проверяем, что документ работает и без приёмника вообще.
        let mut doc = crate::api::buffer::Document::new("начало".as_bytes()).unwrap();
        for _ in 0..9000 {
            doc.insert(doc.len(), b"x").unwrap();
        }
        assert_eq!(
            doc.snapshots_taken(),
            0,
            "документ сам снимает только по прямой просьбе"
        );
        doc.snapshot();
        assert_eq!(
            doc.snapshots_taken(),
            0,
            "без приёмника снимку некуда уйти, он не берётся"
        );
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

    #[test]
    fn the_line_map_stays_in_sync_across_snapshots() {
        // Снимок не меняет текст и не рвёт историю. Карта строк должна
        // совпадать с текстом после каждого шага отмены.
        let tmp = Temp::new("lines-through-ring");
        let file = tmp.file("a.rs");
        let session = tmp.short_session(3, 1_000);
        let mut doc = session.open_file(&file).unwrap();

        // Три строки, между снимками переводы строк двигаются.
        doc.insert(0, "один\nдва\nтри".as_bytes()).unwrap();
        doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(300))));
        doc.snapshot_if_due();

        doc.insert(0, "ноль\n".as_bytes()).unwrap();
        doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(600))));
        doc.snapshot_if_due();

        let before = LineMap::build(&doc.text());
        assert_eq!(
            doc.lines().breaks(),
            before.breaks(),
            "до отката карта жива"
        );

        // Откатываем всё до основания, проверяя карту на каждом шаге.
        let mut undone = false;
        while doc.undo().unwrap().is_some() {
            let fresh = LineMap::build(&doc.text());
            assert_eq!(
                doc.lines().breaks(),
                fresh.breaks(),
                "карта разошлась с текстом {:?}",
                doc.text()
            );
            assert_eq!(doc.line_count(), fresh.line_count());
            undone = true;
        }
        assert!(undone, "откат должен был что-то отменить");
        assert_eq!(doc.text(), Vec::<u8>::new(), "откатилось всё");
        assert_eq!(
            doc.lines().breaks(),
            LineMap::build(&doc.text()).breaks(),
            "после полного отката карта совпадает"
        );
    }
}

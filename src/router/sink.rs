//! Приёмник снапшота: кольцо на диске и один снимок в памяти.
//!
//! Это тот самый стык: документ отдаёт текст и историю, дальше они уходят в
//! sqlite байтами, в том формате, в котором их собрал rope.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::api::buffer::SnapshotSink;
use crate::api::disk::SnapshotStore;

/// Сколько раз пишется снимок, прежде чем признать запись негодной.
///
/// Повтор спасает от сбоя записи и не спасает от порчи извне: если содержимое
/// переписали между попытками, переписывание будет затирать чужое. Разница
/// видна по тому, что именно вернулось, и на пятый раз об этом сказано.
const WRITE_ATTEMPTS: u32 = 5;

/// Приёмник снапшота: кольцо на диске и один снимок в памяти.
///
/// Это тот самый стык: документ отдаёт текст и историю, дальше они уходят в
/// sqlite байтами, в том формате, в котором их собрал rope.
#[derive(Debug)]
pub struct StoreSink {
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
    fn accept(&mut self, file: &Path, text: &[u8], history: &[u8]) -> bool {
        // История идёт вместе с текстом: без неё после перезапуска отменять
        // было бы нечего, то есть снапшот сохранил бы половину работы.
        //
        // Запись проверяется чтением обратно. Если содержимое разошлось, его
        // переписывают: с первого раза это может быть сбой записи, и повтор
        // проходит. Пять попыток подряд — уже не сбой, а порча извне, и
        // перезапись тут бессильна: она затрёт то, что сделали руками.
        let mut last: Option<crate::api::disk::Error> = None;
        for _ in 0..WRITE_ATTEMPTS {
            let seq = match self.store.put(file, text, history) {
                Ok(seq) => seq,
                Err(e) => {
                    last = Some(e);
                    continue;
                }
            };
            match self.read_back(file, seq, text, history) {
                Ok(()) => return true,
                Err(e) => last = Some(e),
            }
        }
        // Пять попыток не помогли. Что именно — известно: ошибка с файлом,
        // номером снимка и обеими суммами, а не «не получилось». Возвращаем
        // `false`: снимок не сохранён, история остаётся в памяти.
        // Сюда можно прийти только с ошибкой: каждая попытка либо вернула
        // `true`, либо записала причину в `last`. Если инвариант когда-нибудь
        // сломают, пользователь не получит выдуманный текст на чужом языке.
        if let Some(error) = last {
            *self.failed.borrow_mut() = Some(error.text());
        }
        false
    }
}

impl StoreSink {
    /// Приёмник поверх своего хранилища и общей ячейки ошибки.
    pub(super) fn new(store: SnapshotStore, failed: Rc<RefCell<Option<String>>>) -> Self {
        Self { store, failed }
    }

    /// Прочитать только что записанный снимок и сверить с тем, что писали.
    ///
    /// Сверку делает хранилище по своей сумме: если содержимое переписали
    /// между записью и чтением, вернётся ошибка порчи, и вызывающий решит,
    /// переписывать или жаловаться.
    fn read_back(
        &self,
        file: &Path,
        seq: u64,
        text: &[u8],
        history: &[u8],
    ) -> Result<(), crate::api::disk::Error> {
        let snapshot =
            self.store
                .get(file, seq)?
                .ok_or_else(|| crate::api::disk::Error::NoSnapshot {
                    file: file.display().to_string(),
                })?;
        if snapshot.text != text || snapshot.history != history {
            return Err(crate::api::disk::Error::Corrupted {
                file: file.display().to_string(),
                seq,
                expected: crate::api::disk::checksum::of_pair(text, history),
                got: snapshot.checksum,
            });
        }
        Ok(())
    }
}

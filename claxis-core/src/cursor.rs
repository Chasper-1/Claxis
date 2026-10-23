//! Курсор и выделение.
//!
//! Курсор — это якорь и голова. Когда они равны, это точка; когда нет —
//! выделение между ними. Никакого отдельного объекта выделения нет.
//!
//! Позиция — байт в документе. Границы графем и токенов считаются на каждом
//! шаге: хранить их между движениями незачем.

use claxis_text::grapheme;

use crate::line::LineMap;
use crate::token::{self, JumpMode, TokenFilter};

/// Байтовая позиция в документе.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position(u32);

impl Position {
    /// Позиция по байтовому смещению.
    pub const fn new(at: u32) -> Self {
        Self(at)
    }

    /// Байтовое смещение.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Смещение для индексации.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<u32> for Position {
    fn from(at: u32) -> Self {
        Self(at)
    }
}

impl From<Position> for u32 {
    fn from(at: Position) -> Self {
        at.0
    }
}

/// Курсор: неподвижный якорь и активная голова.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    anchor: Position,
    head: Position,
    selecting: bool,
}

impl Cursor {
    /// Курсор-точка в позиции `at`.
    pub const fn new(at: Position) -> Self {
        Self {
            anchor: at,
            head: at,
            selecting: false,
        }
    }

    /// Голова — активный конец.
    pub const fn head(self) -> Position {
        self.head
    }

    /// Якорь — неподвижный конец.
    pub const fn anchor(self) -> Position {
        self.anchor
    }

    /// Есть ли выделение.
    pub const fn is_selection(self) -> bool {
        self.anchor.get() != self.head.get()
    }

    /// Границы выделения по возрастанию.
    pub fn range(self) -> (Position, Position) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// Снять выделение, оставив голову.
    pub fn collapse(&mut self) {
        self.anchor = self.head;
        self.selecting = false;
    }

    /// Поставить якорь в текущую голову: дальше движение будет выделять.
    pub fn start_selection(&mut self) {
        self.anchor = self.head;
        self.selecting = true;
    }

    /// Поставить якорь явно.
    pub fn set_anchor(&mut self, at: Position) {
        self.anchor = at;
        self.selecting = true;
    }

    /// Перейти в позицию, сняв выделение.
    pub fn move_to(&mut self, at: Position) {
        self.anchor = at;
        self.head = at;
        self.selecting = false;
    }

    /// Двинуть голову, сохранив якорь. Без выделения якорь идёт следом.
    pub fn move_head(&mut self, at: Position) {
        if !self.selecting {
            self.anchor = at;
        }
        self.head = at;
    }

    /// Двинуть голову, начав выделение, если его ещё нет.
    pub fn select_to(&mut self, at: Position) {
        if !self.selecting {
            self.anchor = self.head;
        }
        self.selecting = true;
        self.head = at;
    }

    /// Вправо на одну графему в пределах строки.
    pub fn grapheme_right(&mut self, line: &[u8], line_start: Position) -> bool {
        let Some(local) = self.local(line, line_start) else {
            return false;
        };
        let end = grapheme::next(line, local);
        self.move_head(Position::new(line_start.get() + end as u32));
        end != local
    }

    /// Влево на одну графему в пределах строки.
    pub fn grapheme_left(&mut self, line: &[u8], line_start: Position) -> bool {
        let Some(local) = self.local(line, line_start) else {
            return false;
        };
        let start = grapheme::prev(line, local);
        self.move_head(Position::new(line_start.get() + start as u32));
        start != local
    }

    /// Вправо по токенам выбранных видов.
    pub fn token_right(
        &mut self,
        line: &[u8],
        line_start: Position,
        filter: TokenFilter,
        mode: JumpMode,
    ) -> bool {
        let Some(local) = self.local(line, line_start) else {
            return false;
        };
        let end = token::next(line, local, filter, mode);
        self.move_head(Position::new(line_start.get() + end as u32));
        end != local
    }

    /// Влево по токенам выбранных видов.
    pub fn token_left(
        &mut self,
        line: &[u8],
        line_start: Position,
        filter: TokenFilter,
        mode: JumpMode,
    ) -> bool {
        let Some(local) = self.local(line, line_start) else {
            return false;
        };
        let start = token::prev(line, local, filter, mode);
        self.move_head(Position::new(line_start.get() + start as u32));
        start != local
    }

    /// Номер графемы от начала строки.
    pub fn grapheme_column(self, line: &[u8], line_start: Position) -> u32 {
        let Some(local) = self.local(line, line_start) else {
            return 0;
        };
        let mut at = 0;
        let mut column = 0;
        while at < local {
            at = grapheme::next(line, at);
            column += 1;
        }
        column
    }

    /// Встать в строку на графему `column`.
    pub fn move_to_column(&mut self, line: &[u8], line_start: Position, column: u32) -> bool {
        let mut at = 0;
        for _ in 0..column {
            let next = grapheme::next(line, at);
            if next == at {
                break;
            }
            at = next;
        }
        let target = Position::new(line_start.get() + at as u32);
        let moved = target != self.head;
        self.move_head(target);
        moved
    }

    /// В начало текущей строки.
    pub fn line_start(&mut self, lines: &LineMap, doc_len: u32) {
        let line = lines.line_of(self.head.get());
        let at = lines.offset_of_line(line, doc_len);
        self.move_head(Position::new(at));
    }

    /// В конец текущей строки, перед переводом строки.
    pub fn line_end(&mut self, lines: &LineMap, doc_len: u32) {
        let line = lines.line_of(self.head.get());
        let at = lines.end_of_line(line, doc_len);
        self.move_head(Position::new(at));
    }

    /// В начало строки `line`.
    pub fn go_to_line(&mut self, line: u32, lines: &LineMap, doc_len: u32) {
        let at = lines.offset_of_line(line, doc_len);
        self.move_head(Position::new(at));
    }

    /// В начало документа.
    pub fn document_start(&mut self) {
        self.move_head(Position::new(0));
    }

    /// В конец документа.
    pub fn document_end(&mut self, doc_len: u32) {
        self.move_head(Position::new(doc_len));
    }

    /// Локальное смещение головы внутри строки.
    fn local(self, line: &[u8], line_start: Position) -> Option<usize> {
        let start = line_start.get();
        let head = self.head.get();
        if head < start {
            return None;
        }
        let local = (head - start) as usize;
        (local <= line.len()).then_some(local)
    }
}

#[cfg(test)]
mod tests {
    use super::{Cursor, Position};
    use crate::line::LineMap;
    use crate::token::{JumpMode, TokenFilter};

    #[test]
    fn a_point_cursor_has_no_selection() {
        let cursor = Cursor::new(Position::new(4));
        assert_eq!(cursor.head(), Position::new(4));
        assert_eq!(cursor.anchor(), Position::new(4));
        assert!(!cursor.is_selection());
        assert_eq!(cursor.range(), (Position::new(4), Position::new(4)));
    }

    #[test]
    fn movement_without_selection_moves_both_ends() {
        let mut cursor = Cursor::new(Position::new(0));
        assert!(cursor.grapheme_right(b"abc", Position::new(0)));
        assert_eq!(cursor.head(), Position::new(1));
        assert_eq!(cursor.anchor(), Position::new(1));
    }

    #[test]
    fn selection_keeps_the_anchor() {
        let mut cursor = Cursor::new(Position::new(1));
        cursor.start_selection();
        assert!(cursor.grapheme_right(b"abc", Position::new(0)));
        assert_eq!(cursor.head(), Position::new(2));
        assert_eq!(cursor.anchor(), Position::new(1));
        assert_eq!(cursor.range(), (Position::new(1), Position::new(2)));
        cursor.collapse();
        assert!(!cursor.is_selection());
        assert_eq!(cursor.anchor(), Position::new(2));
    }

    #[test]
    fn a_combining_mark_is_one_step() {
        let text = "e\u{301}x";
        let mut cursor = Cursor::new(Position::new(0));
        assert!(cursor.grapheme_right(text.as_bytes(), Position::new(0)));
        assert_eq!(cursor.head(), Position::new(3));
        assert!(cursor.grapheme_left(text.as_bytes(), Position::new(0)));
        assert_eq!(cursor.head(), Position::new(0));
    }

    #[test]
    fn token_movement_uses_the_filter() {
        let text = b"foo_bar";
        let mut cursor = Cursor::new(Position::new(0));
        assert!(cursor.token_right(
            text,
            Position::new(0),
            TokenFilter::word(),
            JumpMode::FinishCurrent,
        ));
        assert_eq!(cursor.head(), Position::new(3));
        assert!(cursor.token_right(
            text,
            Position::new(0),
            TokenFilter::symbol(),
            JumpMode::FinishCurrent,
        ));
        assert_eq!(cursor.head(), Position::new(4));
    }

    #[test]
    fn a_combined_filter_moves_over_the_whole_run() {
        let text = b"foo_bar";
        let mut cursor = Cursor::new(Position::new(0));
        let filter = TokenFilter::word().union(TokenFilter::symbol());
        assert!(cursor.token_right(text, Position::new(0), filter, JumpMode::FinishCurrent,));
        assert_eq!(cursor.head(), Position::new(7));
    }

    #[test]
    fn selection_survives_token_movement() {
        let text = b"foo bar";
        let mut cursor = Cursor::new(Position::new(0));
        cursor.start_selection();
        assert!(cursor.token_right(
            text,
            Position::new(0),
            TokenFilter::word(),
            JumpMode::FinishCurrent,
        ));
        assert_eq!(cursor.range(), (Position::new(0), Position::new(3)));
    }

    #[test]
    fn line_edges_are_found_through_the_map() {
        let text = b"one\ntwo\nthree";
        let lines = LineMap::build(text);
        let mut cursor = Cursor::new(Position::new(5));
        cursor.line_start(&lines, text.len() as u32);
        assert_eq!(cursor.head(), Position::new(4));
        cursor.line_end(&lines, text.len() as u32);
        assert_eq!(cursor.head(), Position::new(7));
        cursor.go_to_line(2, &lines, text.len() as u32);
        assert_eq!(cursor.head(), Position::new(8));
        cursor.document_end(text.len() as u32);
        assert_eq!(cursor.head(), Position::new(13));
        cursor.document_start();
        assert_eq!(cursor.head(), Position::new(0));
    }

    #[test]
    fn a_column_is_counted_in_graphemes() {
        let text = "a\u{301}bc";
        let line = text.as_bytes();
        let mut cursor = Cursor::new(Position::new(3));
        assert_eq!(cursor.grapheme_column(line, Position::new(0)), 1);
        cursor.move_to(Position::new(0));
        assert!(cursor.move_to_column(line, Position::new(0), 1));
        assert_eq!(cursor.head(), Position::new(3));
        assert!(cursor.move_to_column(line, Position::new(0), 0));
        assert_eq!(cursor.head(), Position::new(0));
    }

    #[test]
    fn moving_inside_a_point_does_not_leave_a_selection() {
        let text = b"abc";
        let mut cursor = Cursor::new(Position::new(0));
        cursor.select_to(Position::new(2));
        assert!(cursor.is_selection());
        cursor.move_to(Position::new(3));
        assert!(!cursor.is_selection());
        assert_eq!(cursor.head(), Position::new(3));
        cursor.grapheme_left(text, Position::new(0));
        assert!(!cursor.is_selection());
    }
}

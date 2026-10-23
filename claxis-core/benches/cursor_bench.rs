//! Замер прыжков по токенам без кэша.
//!
//! Курсор всегда работает внутри строки: вызывающий отдаёт ему кусок, в
//! котором стоит голова. Поэтому и замер идёт по строкам разной длины, а не по
//! всему документу сразу. Число прыжков фиксировано, чтобы прогоны можно было
//! сравнивать между собой.

use std::hint::black_box;
use std::time::Instant;

use claxis_core::token::{self, JumpMode, TokenFilter};

/// Один кусок смешанного текста: буквы, числа, символы, кириллица, эмодзи.
const PIECE: &str = "foo_bar123 -> привет_мир 3.14 👨‍👩‍👧‍👦 ";

fn line(repeats: usize) -> Vec<u8> {
    PIECE.repeat(repeats).into_bytes()
}

/// Замер вперёд. Курсор упирается в конец строки и начинает сначала.
fn forward(text: &[u8], filter: TokenFilter, rounds: usize, warmup: usize) -> f64 {
    let mut at = 0;
    for _ in 0..warmup {
        at = token::next(black_box(text), at, filter, JumpMode::FinishCurrent);
        if at >= text.len() {
            at = 0;
        }
    }
    let start = Instant::now();
    for _ in 0..rounds {
        at = token::next(black_box(text), at, filter, JumpMode::FinishCurrent);
        if at >= text.len() {
            at = 0;
        }
    }
    start.elapsed().as_nanos() as f64 / rounds as f64
}

/// Замер назад. Курсор упирается в начало строки и начинает с конца.
fn backward(text: &[u8], filter: TokenFilter, rounds: usize, warmup: usize) -> f64 {
    let mut at = text.len();
    for _ in 0..warmup {
        at = token::prev(black_box(text), at, filter, JumpMode::FinishCurrent);
        if at == 0 {
            at = text.len();
        }
    }
    let start = Instant::now();
    for _ in 0..rounds {
        at = token::prev(black_box(text), at, filter, JumpMode::FinishCurrent);
        if at == 0 {
            at = text.len();
        }
    }
    start.elapsed().as_nanos() as f64 / rounds as f64
}

fn report(name: &str, filter: TokenFilter, repeats: usize, rounds: usize, warmup: usize) {
    let text = line(repeats);
    let forward = forward(&text, filter, rounds, warmup);
    let backward = backward(&text, filter, rounds, warmup);
    println!(
        "{name:<18} {:>6} B | {:>8.1} ns вперёд | {:>8.1} ns назад",
        text.len(),
        forward,
        backward
    );
}

fn main() {
    println!("строка              длина   |        вперёд        |         назад");
    report("слова, короткая", TokenFilter::word(), 4, 200_000, 10_000);
    report("слова, средняя", TokenFilter::word(), 32, 100_000, 10_000);
    report("слова, длинная", TokenFilter::word(), 512, 20_000, 10_000);
    report("все виды, короткая", TokenFilter::all(), 4, 200_000, 10_000);
    report("все виды, средняя", TokenFilter::all(), 32, 100_000, 10_000);
    report("все виды, длинная", TokenFilter::all(), 512, 20_000, 10_000);
}

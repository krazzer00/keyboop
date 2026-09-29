//! Перенос `AmbiguousPairs.swift`: спорные пары «латиница ↔ русское слово», набираемые одними
//! клавишами («vs» ↔ «мы»). Выбор человека пишется в `exceptions.force_swap`: одна запись даёт сразу
//! оба поведения (набранное «vs» станет «мы», а набранное «мы» останется).

use crate::exceptions::Exceptions;

pub const PAIRS: &[(&str, &str)] = &[
    ("yt", "не"),
    ("nj", "то"),
    ("yf", "на"),
    ("pf", "за"),
    ("bp", "из"),
    ("lf", "да"),
    ("yb", "ни"),
    ("ns", "ты"),
    ("ds", "вы"),
    ("nf", "та"),
    ("nt", "те"),
    ("vs", "мы"),
    ("here", "руку"),
    ("herb", "руки"),
    ("her", "рук"),
    ("dbl", "вид"),
    ("tt", "ее"),
    ("cj", "со"),
    ("rj", "ко"),
    ("ne", "ту"),
    ("dj", "во"),
    ("tim", "ешь"),
    ("leif", "душа"),
    ("lei", "душ"),
    ("verb", "муки"),
    ("inert", "штуке"),
    ("celt", "суде"),
    ("dyer", "внук"),
    ("buh", "игр"),
    ("lye", "дну"),
    ("neh", "тур"),
    ("vlf", "мда"),
    ("abu", "фиг"),
    ("cv", "см"),
    ("rv", "км"),
    ("ru", "кг"),
    ("rd", "кв"),
    ("vu", "мг"),
    ("uh", "гр"),
    ("in", "шт"),
    ("nsc", "тыс"),
    ("lng", "дтп"),
    ("ids", "швы"),
    ("he", "ру"),
    ("tv", "ем"),
    ("key", "лун"),
    ("keys", "луны"),
    ("ev", "ум"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    En,
    Auto,
    Ru,
}

pub fn choice(exc: &Exceptions, en: &str, ru: &str) -> Choice {
    if exc.is_forced(ru) {
        Choice::Ru
    } else if exc.is_forced(en) {
        Choice::En
    } else {
        Choice::Auto
    }
}

pub fn choose(exc: &mut Exceptions, en: &str, ru: &str, c: Choice) {
    exc.force_swap.remove(en);
    exc.force_swap.remove(ru);
    match c {
        Choice::Ru => exc.add_force_swap(ru),
        Choice::En => exc.add_force_swap(en),
        Choice::Auto => {}
    }
}

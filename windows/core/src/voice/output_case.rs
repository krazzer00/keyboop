//! Перенос `VoiceOutputCase.swift`: «не начинать с заглавной» снимает заглавную только у начала
//! предложения, сохраняя аббревиатуры, частые русские имена (с падежами), слова с намеренным
//! регистром из словаря и имена, уже встреченные с заглавной в середине предложения.

use std::collections::HashSet;
use std::sync::OnceLock;

/// Точка после этих сокращений не завершает предложение.
const NOT_SENTENCE_END: &[&str] = &[
    "г", "гг", "д", "др", "е", "им", "каб", "корп", "кв", "млн", "млрд", "обл", "оф", "п", "пр",
    "проф", "рис", "руб", "с", "см", "стр", "т", "тел", "тыс", "ул", "эт", "mr", "mrs", "ms", "dr",
    "prof", "vs", "fig", "no", "etc", "e", "g", "i",
];

fn fold_token(t: &str) -> String {
    t.to_lowercase().replace('ё', "е")
}

fn is_capitalized(t: &str) -> bool {
    t.chars().next().is_some_and(char::is_uppercase)
}

fn is_acronym(t: &str) -> bool {
    let letters: Vec<char> = t.chars().filter(|c| c.is_alphabetic()).take(2).collect();
    letters.len() == 2 && letters[0].is_uppercase() && letters[1].is_uppercase()
}

/// Снять заглавную у начала предложений. `keeps_case` получает точный целый токен.
pub fn lowercased_sentence_starts(text: &str, keeps_case: &dyn Fn(&str) -> bool) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    let mut at_start = true;
    let mut previous_word = String::new();
    let mut repeated: HashSet<String> = HashSet::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_alphanumeric() {
            let start = i;
            while i < chars.len() && chars[i].is_alphanumeric() {
                i += 1;
            }
            let token: String = chars[start..i].iter().collect();
            let folded = fold_token(&token);
            if at_start {
                let lower = is_capitalized(&token)
                    && !is_acronym(&token)
                    && !keeps_case(&token)
                    && !russian_names().contains(&folded)
                    && !repeated.contains(&folded);
                if lower {
                    let l: Vec<char> = chars[start].to_lowercase().collect();
                    if l.len() == 1 {
                        chars[start] = l[0];
                    }
                }
                at_start = false;
            } else if is_capitalized(&token) {
                repeated.insert(folded);
            }
            previous_word = token;
            continue;
        }
        match c {
            '.' => at_start = !NOT_SENTENCE_END.contains(&fold_token(&previous_word).as_str()),
            '!' | '?' | '…' | '\n' | '\r' => at_start = true,
            ' ' | '\t' | '«' | '»' | '‹' | '›' | '"' | '“' | '”' | '„' | '‘' | '’' | '\'' | '('
            | ')' | '[' | ']' | '{' | '}' | '-' | '—' | '–' | ':' | ';' | ',' => {}
            _ => at_start = false,
        }
        previous_word.clear();
        i += 1;
    }
    chars.into_iter().collect()
}

fn russian_names() -> &'static HashSet<String> {
    static N: OnceLock<HashSet<String>> = OnceLock::new();
    N.get_or_init(build_names)
}

fn build_names() -> HashSet<String> {
    let mut r: HashSet<String> = HashSet::new();
    let mut add = |forms: &[String]| {
        for f in forms {
            r.insert(fold_token(f));
        }
    };
    let drop_last = |s: &str| -> String {
        let mut c: Vec<char> = s.chars().collect();
        c.pop();
        c.into_iter().collect()
    };

    let masc_consonant = |n: &str| {
        vec![
            n.to_string(),
            format!("{n}а"),
            format!("{n}у"),
            format!("{n}ом"),
            format!("{n}е"),
        ]
    };
    for n in [
        "Александр",
        "Альберт",
        "Антон",
        "Артём",
        "Артур",
        "Богдан",
        "Борис",
        "Вадим",
        "Валентин",
        "Виктор",
        "Владимир",
        "Владислав",
        "Вячеслав",
        "Глеб",
        "Даниил",
        "Данил",
        "Денис",
        "Егор",
        "Иван",
        "Кирилл",
        "Константин",
        "Леонид",
        "Макар",
        "Максим",
        "Марк",
        "Михаил",
        "Олег",
        "Роберт",
        "Роман",
        "Руслан",
        "Семён",
        "Станислав",
        "Степан",
        "Фёдор",
        "Эдуард",
        "Ярослав",
        "Яков",
        "Влад",
        "Вадик",
        "Макс",
        "Стас",
    ] {
        add(&masc_consonant(n));
    }
    for n in [
        "Алексей",
        "Анатолий",
        "Андрей",
        "Аркадий",
        "Арсений",
        "Валерий",
        "Василий",
        "Виталий",
        "Геннадий",
        "Георгий",
        "Григорий",
        "Дмитрий",
        "Евгений",
        "Матвей",
        "Николай",
        "Сергей",
        "Тимофей",
        "Юрий",
    ] {
        let stem = drop_last(n);
        let prep = if n.ends_with("ий") {
            format!("{stem}и")
        } else {
            format!("{stem}е")
        };
        add(&[
            n.to_string(),
            format!("{stem}я"),
            format!("{stem}ю"),
            format!("{stem}ем"),
            prep,
        ]);
    }
    for n in ["Игорь", "Лазарь", "Эмиль"] {
        let stem = drop_last(n);
        add(&[
            n.to_string(),
            format!("{stem}я"),
            format!("{stem}ю"),
            format!("{stem}ем"),
            format!("{stem}е"),
        ]);
    }
    for n in [
        "Александра",
        "Алина",
        "Алиса",
        "Алла",
        "Анна",
        "Валентина",
        "Вера",
        "Галина",
        "Диана",
        "Екатерина",
        "Елена",
        "Елизавета",
        "Жанна",
        "Зинаида",
        "Инна",
        "Ирина",
        "Карина",
        "Кира",
        "Кристина",
        "Лариса",
        "Людмила",
        "Маргарита",
        "Марина",
        "Надежда",
        "Нина",
        "Никита",
        "Оксана",
        "Ольга",
        "Полина",
        "Раиса",
        "Светлана",
        "Татьяна",
        "Ульяна",
        "Эмма",
        "Яна",
        "Алёна",
        "Вика",
        "Гена",
        "Гриша",
        "Даша",
        "Дима",
        "Ира",
        "Ксюша",
        "Лена",
        "Лера",
        "Лиза",
        "Лида",
        "Люба",
        "Маша",
        "Миша",
        "Наташа",
        "Нюша",
        "Паша",
        "Рита",
        "Рома",
        "Саша",
        "Света",
        "Слава",
        "Серёжа",
        "Стёпа",
        "Тома",
        "Юра",
        "Яша",
        "Лёша",
        "Сёма",
        "Илюша",
        "Ильюша",
        "Тимоша",
    ] {
        let stem = drop_last(n);
        let hard = stem
            .to_lowercase()
            .chars()
            .last()
            .is_some_and(|c| "гкхжчшщц".contains(c));
        let gen = if hard { "и" } else { "ы" };
        add(&[
            n.to_string(),
            format!("{stem}{gen}"),
            format!("{stem}е"),
            format!("{stem}у"),
            format!("{stem}ой"),
            format!("{stem}ою"),
        ]);
    }
    for n in [
        "Анастасия",
        "Валерия",
        "Виктория",
        "Дарья",
        "Евгения",
        "Зоя",
        "Илья",
        "Ксения",
        "Лидия",
        "Майя",
        "Мария",
        "Наталья",
        "Олеся",
        "София",
        "Юлия",
        "Аня",
        "Боря",
        "Валя",
        "Ваня",
        "Витя",
        "Володя",
        "Галя",
        "Даня",
        "Женя",
        "Катя",
        "Коля",
        "Костя",
        "Лёня",
        "Митя",
        "Надя",
        "Настя",
        "Оля",
        "Петя",
        "Поля",
        "Соня",
        "Таня",
        "Толя",
        "Федя",
        "Юля",
    ] {
        let stem = drop_last(n);
        let dat = if stem.to_lowercase().ends_with('и') {
            "и"
        } else {
            "е"
        };
        add(&[
            n.to_string(),
            format!("{stem}и"),
            format!("{stem}{dat}"),
            format!("{stem}ю"),
            format!("{stem}ей"),
            format!("{stem}ею"),
        ]);
    }
    let fixed: [&[&str]; 6] = [
        &["Павел", "Павла", "Павлу", "Павлом", "Павле"],
        &["Пётр", "Петра", "Петру", "Петром", "Петре"],
        &["Лев", "Льва", "Льву", "Львом", "Льве"],
        &["Любовь", "Любови", "Любовью"],
        &["Мэри", "Николь"],
        &[],
    ];
    for f in fixed {
        add(&f.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    }
    r
}

#[cfg(test)]
mod tests {
    use super::lowercased_sentence_starts as l;

    #[test]
    fn keeps_names_and_acronyms() {
        let none = |_: &str| false;
        assert_eq!(l("Привет. Как дела?", &none), "привет. как дела?");
        assert_eq!(
            l("Роман пришёл. МФЦ закрыт.", &none),
            "Роман пришёл. МФЦ закрыт."
        );
        assert_eq!(l("Это г. Москва", &none), "это г. Москва");
        assert_eq!(
            l("Встретил Эразма. Эразма нет.", &none),
            "встретил Эразма. Эразма нет."
        );
        let keep = |t: &str| t == "Keyboop";
        assert_eq!(l("Keyboop работает", &keep), "Keyboop работает");
    }
}

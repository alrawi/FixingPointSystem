// SPDX-License-Identifier: GPL-3.0-only
//! Prose from small word lists and one sentence grammar per language.
//!
//! Every list in this file was written for this module. A sentence has the
//! form subject, predicate, optional adverbial, optional reference number.
//! The subjects are third person singular noun phrases and the predicates
//! are third person singular present tense, so every combination agrees in
//! person and number without a morphology engine. The sentences make no claim
//! about the world, and no list holds a real person, place, or organisation.

use crate::rng::Rng;

/// The vocabulary and punctuation of one language.
pub(crate) struct Lang {
    pub(crate) code: &'static str,
    subjects: &'static [&'static str],
    predicates: &'static [&'static str],
    adverbials: &'static [&'static str],
    /// Text between two sentences.
    gap: &'static str,
    /// Text that ends a sentence.
    stop: &'static str,
    /// Whether the first letter of a sentence is capitalised.
    capitalise: bool,
    pub(crate) greeting: &'static str,
    pub(crate) closing: &'static str,
}

const EN: Lang = Lang {
    code: "en",
    subjects: &[
        "the quiet river",
        "a small workshop",
        "the northern station",
        "our new library",
        "the old bridge",
        "a patient teacher",
        "the morning market",
        "the local council",
    ],
    predicates: &[
        "carries a long list of notes",
        "opens the doors at dawn",
        "keeps the records in order",
        "welcomes every visitor",
        "repairs the broken clocks",
        "plans a careful survey",
        "shares the results with the group",
        "measures the water each week",
    ],
    adverbials: &[
        " before the winter begins",
        " near the harbour",
        " with great care",
        " every second Tuesday",
        " in the early evening",
        " without any delay",
    ],
    gap: " ",
    stop: ".",
    capitalise: true,
    greeting: "Hello",
    closing: "Kind regards",
};

const DE: Lang = Lang {
    code: "de",
    subjects: &[
        "der ruhige Fluss",
        "eine kleine Werkstatt",
        "die nördliche Station",
        "unsere neue Bibliothek",
        "die alte Brücke",
        "ein geduldiger Lehrer",
        "der Markt am Morgen",
        "der örtliche Rat",
    ],
    predicates: &[
        "führt eine lange Liste von Notizen",
        "öffnet früh am Morgen die Tür",
        "hält alle Unterlagen in Ordnung",
        "begrüßt jeden Besucher",
        "repariert die kaputten Uhren",
        "plant eine sorgfältige Erhebung",
        "teilt die Ergebnisse mit der Gruppe",
        "misst jede Woche das Wasser",
    ],
    adverbials: &[
        " bevor der Winter beginnt",
        " in der Nähe des Hafens",
        " mit großer Sorgfalt",
        " jeden zweiten Dienstag",
        " am frühen Abend",
        " ohne jede Verzögerung",
    ],
    gap: " ",
    stop: ".",
    capitalise: true,
    greeting: "Guten Tag",
    closing: "Mit freundlichen Grüßen",
};

const FR: Lang = Lang {
    code: "fr",
    subjects: &[
        "la rivière tranquille",
        "un petit atelier",
        "la gare du nord",
        "notre nouvelle bibliothèque",
        "le vieux pont",
        "un professeur patient",
        "le marché du matin",
        "le conseil local",
    ],
    predicates: &[
        "porte une longue liste de notes",
        "ouvre ses portes à l'aube",
        "garde les dossiers en ordre",
        "accueille chaque visiteur",
        "répare les horloges cassées",
        "prépare une enquête soignée",
        "partage les résultats avec le groupe",
        "mesure l'eau chaque semaine",
    ],
    adverbials: &[
        " avant le début de l'hiver",
        " près du port",
        " avec beaucoup de soin",
        " un mardi sur deux",
        " en début de soirée",
        " sans aucun retard",
    ],
    gap: " ",
    stop: ".",
    capitalise: true,
    greeting: "Bonjour",
    closing: "Cordialement",
};

const ES: Lang = Lang {
    code: "es",
    subjects: &[
        "el río tranquilo",
        "un pequeño taller",
        "la estación del norte",
        "nuestra nueva biblioteca",
        "el puente viejo",
        "un maestro paciente",
        "el mercado de la mañana",
        "el consejo local",
    ],
    predicates: &[
        "lleva una larga lista de notas",
        "abre sus puertas al amanecer",
        "guarda los registros en orden",
        "recibe a cada visitante",
        "repara los relojes rotos",
        "prepara una encuesta cuidadosa",
        "comparte los resultados con el grupo",
        "mide el agua cada semana",
    ],
    adverbials: &[
        " antes de que empiece el invierno",
        " cerca del puerto",
        " con mucho cuidado",
        " un martes de cada dos",
        " al comienzo de la tarde",
        " sin ningún retraso",
    ],
    gap: " ",
    stop: ".",
    capitalise: true,
    greeting: "Hola",
    closing: "Saludos cordiales",
};

const IT: Lang = Lang {
    code: "it",
    subjects: &[
        "il fiume tranquillo",
        "una piccola officina",
        "la stazione del nord",
        "la nostra nuova biblioteca",
        "il vecchio ponte",
        "un insegnante paziente",
        "il mercato del mattino",
        "il consiglio locale",
    ],
    predicates: &[
        "porta una lunga lista di appunti",
        "apre le sue porte all'alba",
        "tiene i registri in ordine",
        "accoglie ogni visitatore",
        "ripara gli orologi rotti",
        "prepara un'indagine accurata",
        "condivide i risultati con il gruppo",
        "misura l'acqua ogni settimana",
    ],
    adverbials: &[
        " prima che inizi l'inverno",
        " vicino al porto",
        " con grande cura",
        " un martedì su due",
        " all'inizio della sera",
        " senza alcun ritardo",
    ],
    gap: " ",
    stop: ".",
    capitalise: true,
    greeting: "Buongiorno",
    closing: "Cordiali saluti",
};

const RU: Lang = Lang {
    code: "ru",
    subjects: &[
        "тихая река",
        "маленькая мастерская",
        "северная станция",
        "наша новая библиотека",
        "старый мост",
        "терпеливый учитель",
        "утренний рынок",
        "местный совет",
    ],
    predicates: &[
        "несёт длинный список заметок",
        "открывает двери на рассвете",
        "хранит записи в порядке",
        "встречает каждого гостя",
        "чинит сломанные часы",
        "готовит тщательный обзор",
        "делится итогами с группой",
        "каждую неделю измеряет воду",
    ],
    adverbials: &[
        " до начала зимы",
        " возле гавани",
        " с большой осторожностью",
        " каждый второй вторник",
        " ранним вечером",
        " без всякой задержки",
    ],
    gap: " ",
    stop: ".",
    capitalise: true,
    greeting: "Здравствуйте",
    closing: "С уважением",
};

const ZH: Lang = Lang {
    code: "zh",
    subjects: &[
        "安静的河流",
        "小小的工坊",
        "北边的车站",
        "我们的新图书馆",
        "那座老桥",
        "耐心的老师",
        "早晨的集市",
        "当地的委员会",
    ],
    predicates: &[
        "带着一长串笔记",
        "在黎明时打开大门",
        "把记录整理得井井有条",
        "欢迎每一位访客",
        "修理坏掉的钟",
        "计划一次细致的调查",
        "与大家分享结果",
        "每周测量一次水位",
    ],
    adverbials: &[
        "，在冬天开始之前",
        "，在港口附近",
        "，非常仔细",
        "，每隔一个星期二",
        "，在傍晚时分",
        "，没有任何耽搁",
    ],
    gap: "",
    stop: "。",
    capitalise: false,
    greeting: "您好",
    closing: "此致敬礼",
};

/// Every supported language.
pub(crate) const LANGS: &[Lang] = &[EN, DE, FR, ES, IT, RU, ZH];

/// Finds a language by its code.
pub(crate) fn lang(code: &str) -> Option<&'static Lang> {
    LANGS.iter().find(|l| l.code == code)
}

/// Returns `s` with its first letter in upper case.
pub(crate) fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl Lang {
    /// Appends one sentence.
    pub(crate) fn sentence(&self, r: &mut Rng, out: &mut String) {
        let mut s = String::new();
        s.push_str(r.pick(self.subjects));
        if self.gap == " " {
            s.push(' ');
        }
        s.push_str(r.pick(self.predicates));
        if r.chance(2, 3) {
            s.push_str(r.pick(self.adverbials));
        }
        if self.capitalise {
            s = capitalise(&s);
        }
        out.push_str(&s);
        // A reference number in a quarter of the sentences, which keeps short
        // texts from repeating.
        if r.chance(1, 4) {
            out.push_str(&format!(" ({})", r.below(10_000)));
        }
        out.push_str(self.stop);
    }

    /// Returns prose of at least `size` bytes, in paragraphs of three to six
    /// sentences, or an empty string for a size of zero.
    pub(crate) fn prose(&self, r: &mut Rng, size: usize) -> String {
        let mut out = String::with_capacity(size + 256);
        let mut left_in_paragraph = r.range(3, 6);
        while out.trim_end().len() < size {
            self.sentence(r, &mut out);
            left_in_paragraph -= 1;
            if left_in_paragraph == 0 {
                out.push_str("\n\n");
                left_in_paragraph = r.range(3, 6);
            } else {
                out.push_str(self.gap);
            }
        }
        // The text ends with a single line break, not with a blank line.
        let trimmed = out.trim_end_matches([' ', '\n']).len();
        out.truncate(trimmed);
        if !out.is_empty() {
            out.push('\n');
        }
        out
    }
}

/// Syllables for the synthetic personal names. Every name is invented from
/// these syllables and belongs to no one.
const SYLLABLES: &[&str] = &[
    "ka", "lo", "mi", "ra", "ten", "vo", "sa", "du", "fen", "ora", "pel", "zin", "bru", "nae",
    "ilo", "qua",
];

fn word(r: &mut Rng) -> String {
    let mut w = String::new();
    for _ in 0..r.range(2, 3) {
        w.push_str(r.pick(SYLLABLES));
    }
    capitalise(&w)
}

/// Returns an invented first and last name.
pub(crate) fn person(r: &mut Rng) -> (String, String) {
    (word(r), word(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_reaches_the_size_in_every_language() {
        for l in LANGS {
            let mut r = Rng::new(3);
            let p = l.prose(&mut r, 500);
            assert!(p.len() >= 400, "{}: {}", l.code, p.len());
            assert!(p.ends_with('\n') && !p.ends_with("\n\n"), "{}", l.code);
        }
    }

    #[test]
    fn capitalise_handles_multibyte_letters() {
        assert_eq!(capitalise("тихая"), "Тихая");
        assert_eq!(capitalise("der"), "Der");
        assert_eq!(capitalise(""), "");
    }

    #[test]
    fn unknown_languages_are_not_found() {
        assert!(lang("en").is_some());
        assert!(lang("xx").is_none());
    }
}

//! The frames' strings (the desk frame's tab, "Back to you", why a window
//! came forward, the minis' rings; `rune_ui::assistant_overlay::frame` and
//! `pictures`), kept beside [`super::TRANSLATIONS`] as `stage` keeps its
//! own: one row per string with every language, split into the main
//! table's six columns and the Portuguese, French, and Russian tables.

macro_rules! frames_strings {
    ($(($english:literal, [$pl:literal, $zh:literal, $ja:literal, $de:literal, $es:literal, $uk:literal, $pt:literal, $fr:literal, $ru:literal])),* $(,)?) => {
        /// Columns: Polish, Chinese (Simplified), Japanese, German, Spanish, Ukrainian.
        pub(super) const TRANSLATIONS: &[(&str, [&str; 6])] = &[$(($english, [$pl, $zh, $ja, $de, $es, $uk])),*];
        /// Brazilian Portuguese.
        pub(super) const PORTUGUESE: &[(&str, &str)] = &[$(($english, $pt)),*];
        pub(super) const FRENCH: &[(&str, &str)] = &[$(($english, $fr)),*];
        pub(super) const RUSSIAN: &[(&str, &str)] = &[$(($english, $ru)),*];
    };
}

#[rustfmt::skip]
frames_strings![
    ("Agent-Desk is using {} for a moment", ["Agent-Desk używa przez chwilę: {}", "Agent-Desk 正在短暂使用 {}", "Agent-Desk が少しの間 {} を使っています", "Agent-Desk nutzt kurz {}", "Agent-Desk usa {} un momento", "Agent-Desk ненадовго використовує {}", "O Agent-Desk está usando o {} por um momento", "Agent-Desk utilise {} un instant", "Agent-Desk ненадолго использует {}"]),
    ("Agent-Desk is using your mouse and keys in {}", ["Agent-Desk używa twojej myszy i klawiszy w {}", "Agent-Desk 正在 {} 中使用你的鼠标和键盘", "Agent-Desk が {} であなたのマウスとキーを使っています", "Agent-Desk nutzt deine Maus und Tastatur in {}", "Agent-Desk usa tu ratón y teclas en {}", "Agent-Desk використовує вашу мишу й клавіші в {}", "O Agent-Desk está usando seu mouse e teclas no {}", "Agent-Desk utilise votre souris et vos touches dans {}", "Agent-Desk использует вашу мышь и клавиши в {}"]),
    ("Agent-Desk is using your mouse and keys", ["Agent-Desk używa twojej myszy i klawiszy", "Agent-Desk 正在使用你的鼠标和键盘", "Agent-Desk があなたのマウスとキーを使っています", "Agent-Desk nutzt deine Maus und Tastatur", "Agent-Desk usa tu ratón y teclas", "Agent-Desk використовує вашу мишу й клавіші", "O Agent-Desk está usando seu mouse e teclas", "Agent-Desk utilise votre souris et vos touches", "Agent-Desk использует вашу мышь и клавиши"]),
    ("back in {} s", ["wróci za {} s", "{} 秒后归还", "{} 秒で戻ります", "zurück in {} s", "vuelve en {} s", "поверне за {} с", "volta em {} s", "retour dans {} s", "вернёт через {} с"]),
    ("Back to you · {}", ["Znów twoje · {}", "交还给你 · {}", "あなたに戻りました · {}", "Zurück bei dir · {}", "De vuelta contigo · {}", "Знову ваше · {}", "De volta para você · {}", "À vous de nouveau · {}", "Снова ваше · {}"]),
    ("Touch anything to take over", ["Dotknij czegokolwiek, by przejąć", "触碰任意处即可接管", "何かに触れると引き継げます", "Berühre etwas, um zu übernehmen", "Toca cualquier cosa para tomar el control", "Торкніться будь-чого, щоб перейняти", "Toque em qualquer coisa para assumir", "Touchez n'importe quoi pour reprendre la main", "Коснитесь чего угодно, чтобы перехватить"]),
    ("{} didn't respond in the background", ["{} nie reagował w tle", "{} 在后台没有响应", "{} がバックグラウンドで応答しませんでした", "{} hat im Hintergrund nicht reagiert", "{} no respondió en segundo plano", "{} не відповідав у фоні", "O {} não respondeu em segundo plano", "{} n'a pas répondu en arrière-plan", "{} не отвечал в фоне"]),
    ("{} needs keys typed in front", ["{} potrzebuje klawiszy na pierwszym planie", "{} 需要在前台输入按键", "{} は前面でのキー入力が必要です", "{} braucht Tastatureingaben im Vordergrund", "{} necesita teclas en primer plano", "{} потребує клавіш на передньому плані", "O {} precisa de teclas em primeiro plano", "{} a besoin de touches au premier plan", "{} нужны клавиши на переднем плане"]),
    ("acting", ["działa", "正在操作", "操作中", "aktiv", "actuando", "діє", "agindo", "en action", "действует"]),
];

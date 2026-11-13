//! The island's strings (the black capsule at the notch, its live words
//! while the Action shortcut is held, and its reasoning lines;
//! `rune_ui::assistant_overlay::island`), and the thinking line a quick
//! turn says when it stays silent (`assistant::service::voice`), kept
//! beside [`super::TRANSLATIONS`] as `stage` keeps its own: one row per
//! string with every language, split into the main table's six columns and
//! the Portuguese, French, and Russian tables.

macro_rules! island_strings {
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
island_strings![
    ("On it.", ["Już się tym zajmuję.", "这就去办。", "了解、やります。", "Mach ich.", "Me pongo con ello.", "Беруся.", "Deixa comigo.", "Je m'en occupe.", "Сейчас сделаю."]),
    ("Understood", ["Zrozumiałem", "已理解", "理解した内容", "Verstanden", "Entendido", "Зрозумів", "Entendido", "Compris", "Понял"]),
    ("Doing now", ["Teraz robię", "正在做", "いま実行中", "Gerade", "Haciendo ahora", "Зараз роблю", "Fazendo agora", "En cours", "Сейчас делаю"]),
    ("Up next", ["Następnie", "接下来", "次に", "Als Nächstes", "A continuación", "Далі", "Em seguida", "Ensuite", "Дальше"]),
    ("Understood: {}", ["Zrozumiałem: {}", "已理解：{}", "理解：{}", "Verstanden: {}", "Entendido: {}", "Зрозумів: {}", "Entendido: {}", "Compris : {}", "Понял: {}"]),
    ("{} in front", ["{} na wierzchu", "{} 在前台", "{} が前面", "{} vorne", "{} delante", "{} на передньому плані", "{} na frente", "{} au premier plan", "{} на переднем плане"]),
    ("Listening · {} is in front", ["Słucham · {} jest na wierzchu", "正在聆听 · {} 在前台", "聞き取り中 · {} が前面", "Hört zu · {} ist vorne", "Escuchando · {} está delante", "Слухаю · {} на передньому плані", "Ouvindo · {} está na frente", "À l'écoute · {} est au premier plan", "Слушаю · {} на переднем плане"]),
    ("Release {} to send · Esc to cancel", ["Puść {}, aby wysłać · Esc anuluje", "松开 {} 发送 · Esc 取消", "{} を離すと送信 · Esc でキャンセル", "{} loslassen zum Senden · Esc bricht ab", "Suelta {} para enviar · Esc cancela", "Відпустіть {}, щоб надіслати · Esc скасовує", "Solte {} para enviar · Esc cancela", "Relâchez {} pour envoyer · Échap annule", "Отпустите {}, чтобы отправить · Esc отменяет"]),
    ("Hold {} to change it · Esc stops", ["Przytrzymaj {}, aby to zmienić · Esc zatrzymuje", "按住 {} 可更改 · Esc 停止", "{} を長押しで変更 · Esc で停止", "{} halten, um es zu ändern · Esc stoppt", "Mantén {} para cambiarlo · Esc lo detiene", "Утримуйте {}, щоб змінити · Esc зупиняє", "Segure {} para mudar · Esc para", "Maintenez {} pour le changer · Échap arrête", "Удерживайте {}, чтобы изменить · Esc останавливает"]),
    ("Say what you need", ["Powiedz, czego potrzebujesz", "说出你需要什么", "用件を話してください", "Sag, was du brauchst", "Di lo que necesitas", "Скажіть, що вам потрібно", "Diga do que precisa", "Dites ce qu'il vous faut", "Скажите, что нужно"]),
];

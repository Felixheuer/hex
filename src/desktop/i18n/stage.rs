//! The stage's strings (long answers, results, What it did, the repair
//! card; `rune_ui::assistant_overlay::stage`), kept beside
//! [`super::TRANSLATIONS`] as `assistant_overlay` keeps its own: one row
//! per string with every language, split into the main table's six columns
//! and the Portuguese, French, and Russian tables.

macro_rules! stage_strings {
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
stage_strings![
    ("Show", ["Pokaż", "显示", "表示", "Zeigen", "Mostrar", "Показати", "Mostrar", "Afficher", "Показать"]),
    ("Open link", ["Otwórz link", "打开链接", "リンクを開く", "Link öffnen", "Abrir enlace", "Відкрити посилання", "Abrir link", "Ouvrir le lien", "Открыть ссылку"]),
    ("Open in Finder", ["Otwórz w Finderze", "在访达中打开", "Finder で開く", "Im Finder öffnen", "Abrir en Finder", "Відкрити у Finder", "Abrir no Finder", "Ouvrir dans le Finder", "Открыть в Finder"]),
    ("Folder", ["Folder", "文件夹", "フォルダ", "Ordner", "Carpeta", "Папка", "Pasta", "Dossier", "Папка"]),
    ("Pin", ["Przypnij", "固定", "固定", "Anheften", "Fijar", "Закріпити", "Fixar", "Épingler", "Закрепить"]),
    ("Unpin", ["Odepnij", "取消固定", "固定を解除", "Lösen", "Soltar", "Відкріпити", "Desafixar", "Désépingler", "Открепить"]),
    ("Pinned", ["Przypięte", "已固定", "固定中", "Angeheftet", "Fijado", "Закріплено", "Fixado", "Épinglé", "Закреплено"]),
    ("What it did", ["Co zrobił", "做了什么", "やったこと", "Was es getan hat", "Qué hizo", "Що зроблено", "O que fez", "Ce qu'il a fait", "Что сделано"]),
    ("Needs you", ["Potrzebuje ciebie", "需要你", "あなたが必要", "Braucht dich", "Te necesita", "Потрібні ви", "Precisa de você", "A besoin de vous", "Нужны вы"]),
    ("Fix all…", ["Napraw wszystko…", "全部修复…", "すべて修正…", "Alles beheben…", "Arreglar todo…", "Виправити все…", "Corrigir tudo…", "Tout corriger…", "Исправить всё…"]),
    ("Save as note", ["Zapisz jako notatkę", "存为笔记", "メモとして保存", "Als Notiz sichern", "Guardar como nota", "Зберегти як нотатку", "Salvar como nota", "Enregistrer en note", "Сохранить как заметку"]),
    ("Open in Agent-Desk", ["Otwórz w Agent-Desk", "在 Agent-Desk 中打开", "Agent-Desk で開く", "In Agent-Desk öffnen", "Abrir en Agent-Desk", "Відкрити в Agent-Desk", "Abrir no Agent-Desk", "Ouvrir dans Agent-Desk", "Открыть в Agent-Desk"]),
    ("Esc puts it away", ["Esc chowa", "按 Esc 收起", "Esc で閉じる", "Esc legt es weg", "Esc lo guarda", "Esc ховає", "Esc fecha", "Échap le range", "Esc скрывает"]),
    ("Hold {} to ask more", ["Przytrzymaj {}, aby zapytać o więcej", "按住 {} 继续提问", "{} を長押ししてさらに質問", "{} halten, um mehr zu fragen", "Mantén {} para preguntar más", "Утримуйте {}, щоб запитати ще", "Segure {} para perguntar mais", "Maintenez {} pour en demander plus", "Удерживайте {}, чтобы спросить ещё"]),
    ("Worked in {}", ["Pracował w {}", "在 {} 中工作", "{} で作業", "In {} gearbeitet", "Trabajó en {}", "Працював у {}", "Trabalhou no {}", "A travaillé dans {}", "Работал в {}"]),
    ("Agent-Desk needs your help", ["Agent-Desk potrzebuje twojej pomocy", "Agent-Desk 需要你的帮助", "Agent-Desk に手助けが必要です", "Agent-Desk braucht deine Hilfe", "Agent-Desk necesita tu ayuda", "Agent-Desk потребує вашої допомоги", "O Agent-Desk precisa da sua ajuda", "Agent-Desk a besoin de votre aide", "Agent-Desk нужна ваша помощь"]),
    ("No steps were recorded for this task.", ["Dla tego zadania nie zapisano kroków.", "此任务没有记录步骤。", "このタスクの手順は記録されていません。", "Für diesen Auftrag wurden keine Schritte aufgezeichnet.", "No se registraron pasos para esta tarea.", "Для цього завдання кроки не записано.", "Nenhuma etapa foi registrada para esta tarefa.", "Aucune étape n'a été enregistrée pour cette tâche.", "Для этой задачи шаги не записаны."]),
    ("Say “open the folder”", ["Powiedz „otwórz folder”", "说“打开文件夹”", "「フォルダを開いて」と言う", "Sag „öffne den Ordner“", "Di «abre la carpeta»", "Скажіть «відкрий папку»", "Diga “abra a pasta”", "Dites « ouvre le dossier »", "Скажите «открой папку»"]),
    ("Say “open the file”", ["Powiedz „otwórz plik”", "说“打开文件”", "「ファイルを開いて」と言う", "Sag „öffne die Datei“", "Di «abre el archivo»", "Скажіть «відкрий файл»", "Diga “abra o arquivo”", "Dites « ouvre le fichier »", "Скажите «открой файл»"]),
    ("Say “open the link”", ["Powiedz „otwórz link”", "说“打开链接”", "「リンクを開いて」と言う", "Sag „öffne den Link“", "Di «abre el enlace»", "Скажіть «відкрий посилання»", "Diga “abra o link”", "Dites « ouvre le lien »", "Скажите «открой ссылку»"]),
    ("Open Agent-Desk", ["Otwórz Agent-Desk", "打开 Agent-Desk", "Agent-Desk を開く", "Agent-Desk öffnen", "Abrir Agent-Desk", "Відкрити Agent-Desk", "Abrir o Agent-Desk", "Ouvrir Agent-Desk", "Открыть Agent-Desk"]),
];

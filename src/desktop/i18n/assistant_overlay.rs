//! The assistant's overlay strings (the orb and its list, the Here bubble,
//! the hint past the HUD, and the Hands off pill), kept beside
//! [`super::TRANSLATIONS`]. One row per string with every language, so none
//! is ever left out; the macro splits them into the main table's six
//! columns and the Portuguese, French, and Russian tables.

macro_rules! overlay_strings {
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
overlay_strings![
    ("Thinking…", ["Myślę…", "正在思考…", "考え中…", "Denkt nach…", "Pensando…", "Думаю…", "Pensando…", "Réflexion…", "Думаю…"]),
    ("Open in Chats", ["Otwórz w czatach", "在聊天中打开", "チャットで開く", "In Chats öffnen", "Abrir en Chats", "Відкрити в чатах", "Abrir nos chats", "Ouvrir dans les discussions", "Открыть в чатах"]),
    ("Queued", ["W kolejce", "排队中", "待機中", "In der Warteschlange", "En cola", "У черзі", "Na fila", "En attente", "В очереди"]),
    ("Stopped", ["Zatrzymano", "已停止", "停止しました", "Gestoppt", "Detenida", "Зупинено", "Interrompida", "Arrêtée", "Остановлено"]),
    ("Stop task", ["Zatrzymaj zadanie", "停止任务", "タスクを停止", "Aufgabe stoppen", "Detener tarea", "Зупинити завдання", "Parar tarefa", "Arrêter la tâche", "Остановить задачу"]),
    ("Assistant tasks", ["Zadania asystenta", "助手任务", "アシスタントのタスク", "Aufgaben des Assistenten", "Tareas del asistente", "Завдання асистента", "Tarefas do assistente", "Tâches de l'assistant", "Задачи ассистента"]),
    ("Assistant tasks: {}", ["Zadania asystenta: {}", "助手任务：{}", "アシスタントのタスク: {}", "Aufgaben des Assistenten: {}", "Tareas del asistente: {}", "Завдання асистента: {}", "Tarefas do assistente: {}", "Tâches de l'assistant : {}", "Задачи ассистента: {}"]),
    ("New background task", ["Nowe zadanie w tle", "新的后台任务", "新しいバックグラウンドタスク", "Neue Hintergrundaufgabe", "Nueva tarea en segundo plano", "Нове фонове завдання", "Nova tarefa em segundo plano", "Nouvelle tâche en arrière-plan", "Новая фоновая задача"]),
    ("Here · {}", ["Tutaj · {}", "此处 · {}", "ここ · {}", "Hier · {}", "Aquí · {}", "Тут · {}", "Aqui · {}", "Ici · {}", "Здесь · {}"]),
    ("You have it", ["Masz kontrolę", "由你操作", "あなたが操作中", "Du hast übernommen", "Lo tienes tú", "Керуєте ви", "Com você", "C'est à vous", "Управляете вы"]),
    ("Take over", ["Przejmij", "接管", "引き継ぐ", "Übernehmen", "Tomar el control", "Перехопити", "Assumir", "Prendre la main", "Перехватить"]),
    ("Hand back", ["Oddaj", "交还", "返す", "Zurückgeben", "Devolver", "Повернути", "Devolver", "Rendre la main", "Вернуть"]),
    ("Waiting for limit · resumes {}", ["Czeka na limit · wznowi o {}", "等待额度 · {} 恢复", "上限の解除待ち · {} に再開", "Wartet auf das Limit · weiter um {}", "Esperando el límite · se reanuda a las {}", "Чекає на ліміт · продовжить о {}", "Aguardando o limite · retoma às {}", "En attente de la limite · reprise à {}", "Ждёт лимит · продолжит в {}"]),
    ("Hands off: Hex is using your mouse", ["Nie dotykaj: Hex używa twojej myszy", "请勿操作：Hex 正在使用你的鼠标", "操作しないでください：Hex がマウスを使用中", "Finger weg: Hex benutzt deine Maus", "No toques nada: Hex está usando tu ratón", "Не торкайтеся: Hex використовує вашу мишу", "Não mexa: o Hex está usando seu mouse", "Ne touchez à rien : Hex utilise votre souris", "Не трогайте: Hex использует вашу мышь"]),
    ("Esc to stop", ["Esc, aby zatrzymać", "按 Esc 停止", "Esc で停止", "Esc zum Stoppen", "Esc para detener", "Esc, щоб зупинити", "Esc para parar", "Échap pour arrêter", "Esc — остановить"]),
    ("On the web", ["W sieci", "在网页上", "ウェブ上", "Im Web", "En la web", "В інтернеті", "Na web", "Sur le web", "В интернете"]),
    ("Finish setting up the assistant first", ["Najpierw dokończ konfigurację asystenta", "请先完成助手设置", "先にアシスタントの設定を完了してください", "Richte zuerst den Assistenten fertig ein", "Primero termina de configurar el asistente", "Спершу завершіть налаштування асистента", "Primeiro termine de configurar o assistente", "Terminez d'abord la configuration de l'assistant", "Сначала завершите настройку ассистента"]),
    ("Continue setup", ["Kontynuuj konfigurację", "继续设置", "設定を続ける", "Einrichtung fortsetzen", "Continuar la configuración", "Продовжити налаштування", "Continuar configuração", "Continuer la configuration", "Продолжить настройку"]),
];

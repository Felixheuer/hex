//! Labs › Assistant's overlay strings (the task stack, the quick answer,
//! and "New task" beside the HUD), kept beside [`super::TRANSLATIONS`] with
//! their Portuguese, French, and Russian, which those tables take in, so
//! the assistant's rows stay together.

/// Columns: Polish, Chinese (Simplified), Japanese, German, Spanish, Ukrainian.
#[rustfmt::skip]
pub(super) const TRANSLATIONS: &[(&str, [&str; 6])] = &[
    ("Thinking…", ["Myślę…", "正在思考…", "考え中…", "Denkt nach…", "Pensando…", "Думаю…"]),
    ("Open in Chats", ["Otwórz w czatach", "在聊天中打开", "チャットで開く", "In Chats öffnen", "Abrir en Chats", "Відкрити в чатах"]),
    ("Queued", ["W kolejce", "排队中", "待機中", "In der Warteschlange", "En cola", "У черзі"]),
    ("Stopped", ["Zatrzymano", "已停止", "停止しました", "Gestoppt", "Detenida", "Зупинено"]),
    ("Stop task", ["Zatrzymaj zadanie", "停止任务", "タスクを停止", "Aufgabe stoppen", "Detener tarea", "Зупинити завдання"]),
    ("+{} more", ["+{} więcej", "另外 {} 个", "他 {} 件", "+{} weitere", "+{} más", "+{} ще"]),
    ("Assistant tasks", ["Zadania asystenta", "助手任务", "アシスタントのタスク", "Aufgaben des Assistenten", "Tareas del asistente", "Завдання асистента"]),
    ("New task", ["Nowe zadanie", "新任务", "新しいタスク", "Neue Aufgabe", "Nueva tarea", "Нове завдання"]),
];

#[rustfmt::skip]
pub(super) const PORTUGUESE: &[(&str, &str)] = &[
    ("Thinking…", "Pensando…"),
    ("Open in Chats", "Abrir nos chats"),
    ("Queued", "Na fila"),
    ("Stopped", "Interrompida"),
    ("Stop task", "Parar tarefa"),
    ("+{} more", "+{} mais"),
    ("Assistant tasks", "Tarefas do assistente"),
    ("New task", "Nova tarefa"),
];

#[rustfmt::skip]
pub(super) const FRENCH: &[(&str, &str)] = &[
    ("Thinking…", "Réflexion…"),
    ("Open in Chats", "Ouvrir dans les discussions"),
    ("Queued", "En attente"),
    ("Stopped", "Arrêtée"),
    ("Stop task", "Arrêter la tâche"),
    ("+{} more", "+{} de plus"),
    ("Assistant tasks", "Tâches de l'assistant"),
    ("New task", "Nouvelle tâche"),
];

#[rustfmt::skip]
pub(super) const RUSSIAN: &[(&str, &str)] = &[
    ("Thinking…", "Думаю…"),
    ("Open in Chats", "Открыть в чатах"),
    ("Queued", "В очереди"),
    ("Stopped", "Остановлено"),
    ("Stop task", "Остановить задачу"),
    ("+{} more", "ещё {}"),
    ("Assistant tasks", "Задачи ассистента"),
    ("New task", "Новая задача"),
];

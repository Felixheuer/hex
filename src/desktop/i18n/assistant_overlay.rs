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
    ("Live browsers", ["Przeglądarki na żywo", "实时浏览器", "ライブのブラウザ", "Live-Browser", "Navegadores en vivo", "Браузери наживо", "Navegadores ao vivo", "Navigateurs en direct", "Браузеры вживую"]),
    ("Live browser", ["Przeglądarka na żywo", "实时浏览器", "ライブのブラウザ", "Live-Browser", "Navegador en vivo", "Браузер наживо", "Navegador ao vivo", "Navigateur en direct", "Браузер вживую"]),
    ("Main browser", ["Główna przeglądarka", "主浏览器", "メインのブラウザ", "Hauptbrowser", "Navegador principal", "Основний браузер", "Navegador principal", "Navigateur principal", "Основной браузер"]),
    ("Open {} live", ["Otwórz {} na żywo", "实时打开 {}", "{} をライブで開く", "{} live öffnen", "Abrir {} en vivo", "Відкрити {} наживо", "Abrir {} ao vivo", "Ouvrir {} en direct", "Открыть {} вживую"]),
    ("Loading…", ["Wczytywanie…", "正在加载…", "読み込み中…", "Lädt…", "Cargando…", "Завантаження…", "Carregando…", "Chargement…", "Загрузка…"]),
    ("No live picture", ["Brak obrazu na żywo", "没有实时画面", "ライブ画像なし", "Kein Live-Bild", "Sin imagen en vivo", "Немає живого зображення", "Sem imagem ao vivo", "Pas d'image en direct", "Нет живого изображения"]),
    ("No live picture of this browser", ["Brak obrazu na żywo z tej przeglądarki", "此浏览器没有实时画面", "このブラウザのライブ画像はありません", "Von diesem Browser gibt es kein Live-Bild", "No hay imagen en vivo de este navegador", "Цей браузер не має живого зображення", "Não há imagem ao vivo deste navegador", "Pas d'image en direct de ce navigateur", "У этого браузера нет живого изображения"]),
    ("Live browser you're driving", ["Przeglądarka na żywo, którą sterujesz", "你正在操作的实时浏览器", "あなたが操作中のライブのブラウザ", "Live-Browser, den du steuerst", "Navegador en vivo que controlas", "Браузер наживо, яким керуєте ви", "Navegador ao vivo que você controla", "Navigateur en direct que vous pilotez", "Браузер вживую, которым управляете вы"]),
    ("You're driving: click, scroll, and type in the page. The task waits.", ["Sterujesz: klikaj, przewijaj i pisz na stronie. Zadanie czeka.", "由你操作：在页面中点击、滚动和输入。任务会等待。", "あなたが操作中です。ページでクリック、スクロール、入力できます。タスクは待機します。", "Du steuerst: Klicke, scrolle und tippe in der Seite. Die Aufgabe wartet.", "Tienes el control: haz clic, desplázate y escribe en la página. La tarea espera.", "Керуєте ви: клацайте, гортайте й друкуйте на сторінці. Завдання чекає.", "Você está no controle: clique, role e digite na página. A tarefa espera.", "C'est vous qui pilotez : cliquez, faites défiler et tapez dans la page. La tâche attend.", "Управляете вы: нажимайте, прокручивайте и печатайте на странице. Задача ждёт."]),
    ("The assistant is working here. Take over to drive it yourself.", ["Asystent tu pracuje. Przejmij, aby sterować samodzielnie.", "助手正在这里工作。接管即可自己操作。", "アシスタントがここで作業中です。引き継ぐと自分で操作できます。", "Der Assistent arbeitet hier. Übernimm, um selbst zu steuern.", "El asistente está trabajando aquí. Toma el control para manejarlo tú.", "Асистент працює тут. Перехопіть, щоб керувати самостійно.", "O assistente está trabalhando aqui. Assuma para controlar você mesmo.", "L'assistant travaille ici. Prenez la main pour piloter vous-même.", "Ассистент работает здесь. Перехватите, чтобы управлять самостоятельно."]),
    ("This task has finished.", ["To zadanie się zakończyło.", "此任务已完成。", "このタスクは終了しました。", "Diese Aufgabe ist beendet.", "Esta tarea ha terminado.", "Це завдання завершено.", "Esta tarefa terminou.", "Cette tâche est terminée.", "Эта задача завершена."]),
    ("Take over", ["Przejmij", "接管", "引き継ぐ", "Übernehmen", "Tomar el control", "Перехопити", "Assumir", "Prendre la main", "Перехватить"]),
    ("Hand back", ["Oddaj", "交还", "返す", "Zurückgeben", "Devolver", "Повернути", "Devolver", "Rendre la main", "Вернуть"]),
    ("Waiting for limit · resumes {}", ["Czeka na limit · wznowi o {}", "等待额度 · {} 恢复", "上限の解除待ち · {} に再開", "Wartet auf das Limit · weiter um {}", "Esperando el límite · se reanuda a las {}", "Чекає на ліміт · продовжить о {}", "Aguardando o limite · retoma às {}", "En attente de la limite · reprise à {}", "Ждёт лимит · продолжит в {}"]),
    ("Hands off: Hex is using your mouse", ["Nie dotykaj: Hex używa twojej myszy", "请勿操作：Hex 正在使用你的鼠标", "操作しないでください：Hex がマウスを使用中", "Finger weg: Hex benutzt deine Maus", "No toques nada: Hex está usando tu ratón", "Не торкайтеся: Hex використовує вашу мишу", "Não mexa: o Hex está usando seu mouse", "Ne touchez à rien : Hex utilise votre souris", "Не трогайте: Hex использует вашу мышь"]),
    ("Esc to stop", ["Esc, aby zatrzymać", "按 Esc 停止", "Esc で停止", "Esc zum Stoppen", "Esc para detener", "Esc, щоб зупинити", "Esc para parar", "Échap pour arrêter", "Esc — остановить"]),
    ("On the web", ["W sieci", "在网页上", "ウェブ上", "Im Web", "En la web", "В інтернеті", "Na web", "Sur le web", "В интернете"]),
    ("{}: show every card", ["{}: pokaż wszystkie karty", "{}：显示全部卡片", "{}：すべてのカードを表示", "{}: alle Karten zeigen", "{}: mostrar todas las tarjetas", "{}: показати всі картки", "{}: mostrar todos os cartões", "{} : afficher toutes les cartes", "{}: показать все карточки"]),
    ("{}: hide the cards", ["{}: ukryj karty", "{}：隐藏卡片", "{}：カードを隠す", "{}: Karten ausblenden", "{}: ocultar las tarjetas", "{}: сховати картки", "{}: ocultar os cartões", "{} : masquer les cartes", "{}: скрыть карточки"]),
    ("Open the task list", ["Otwórz listę zadań", "打开任务列表", "タスク一覧を開く", "Aufgabenliste öffnen", "Abrir la lista de tareas", "Відкрити список завдань", "Abrir a lista de tarefas", "Ouvrir la liste des tâches", "Открыть список задач"]),
    ("{} d", ["{} dni", "{} 天", "{} 日", "{} Tg.", "{} d", "{} дн.", "{} d", "{} j", "{} дн."]),
    ("{} jobs wait for the limit", ["Zadania czekające na limit: {}", "{} 个任务在等待额度", "{} 件が上限の解除待ち", "{} Aufträge warten auf das Limit", "{} trabajos esperan el límite", "Завдань чекає на ліміт: {}", "{} trabalhos aguardam o limite", "{} tâches attendent la limite", "Задач ждут лимит: {}"]),
    ("resumes {}", ["wznowi o {}", "{} 恢复", "{} に再開", "weiter um {}", "se reanuda a las {}", "продовжить о {}", "retoma às {}", "reprise à {}", "продолжит в {}"]),
    ("{} jobs running", ["Działające zadania: {}", "{} 个任务正在运行", "{} 件の仕事が進行中", "{} Aufträge laufen", "{} trabajos en curso", "Виконується завдань: {}", "{} trabalhos em andamento", "{} tâches en cours", "Выполняется задач: {}"]),
    ("Finish setting up the assistant first", ["Najpierw dokończ konfigurację asystenta", "请先完成助手设置", "先にアシスタントの設定を完了してください", "Richte zuerst den Assistenten fertig ein", "Primero termina de configurar el asistente", "Спершу завершіть налаштування асистента", "Primeiro termine de configurar o assistente", "Terminez d'abord la configuration de l'assistant", "Сначала завершите настройку ассистента"]),
    ("{} asks", ["{} pyta", "{} 提问", "{} からの質問", "{} fragt", "{} pregunta", "{} питає", "{} pergunta", "{} demande", "{} спрашивает"]),
    ("Open questions: {}", ["Otwarte pytania: {}", "待回答的问题：{}", "未回答の質問：{}", "Offene Fragen: {}", "Preguntas abiertas: {}", "Відкриті запитання: {}", "Perguntas em aberto: {}", "Questions en attente : {}", "Открытые вопросы: {}"]),
    ("Waiting for you", ["Czeka na ciebie", "等你回复", "あなたの返答待ち", "Wartet auf dich", "Esperándote", "Чекає на вас", "Esperando por você", "En attente de vous", "Ждёт вас"]),
    ("Hold {} to answer", ["Przytrzymaj {}, aby odpowiedzieć", "按住 {} 回答", "{} を長押しして回答", "Halte {} zum Antworten", "Mantén {} para responder", "Утримуйте {}, щоб відповісти", "Segure {} para responder", "Maintenez {} pour répondre", "Удерживайте {}, чтобы ответить"]),
    ("Hide question", ["Ukryj pytanie", "隐藏问题", "質問を隠す", "Frage ausblenden", "Ocultar pregunta", "Сховати запитання", "Ocultar pergunta", "Masquer la question", "Скрыть вопрос"]),
    ("Waiting for your answer", ["Czeka na twoją odpowiedź", "等待你的回答", "回答を待っています", "Wartet auf deine Antwort", "Esperando tu respuesta", "Чекає на вашу відповідь", "Aguardando sua resposta", "En attente de votre réponse", "Ждёт вашего ответа"]),
    ("{} done", ["Gotowe: {}", "已完成 {} 项", "{} 件完了", "{} erledigt", "{} hechos", "Виконано: {}", "{} concluídos", "{} terminées", "Готово: {}"]),
    ("+{} more", ["+{} więcej", "另外 {} 个", "他 {} 件", "+{} weitere", "+{} más", "+{} ще", "+{} mais", "+{} de plus", "ещё {}"]),
    ("Continue setup", ["Kontynuuj konfigurację", "继续设置", "設定を続ける", "Einrichtung fortsetzen", "Continuar la configuración", "Продовжити налаштування", "Continuar configuração", "Continuer la configuration", "Продолжить настройку"]),
];

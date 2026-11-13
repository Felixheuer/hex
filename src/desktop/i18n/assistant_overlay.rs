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
    ("Show in Assistant", ["Pokaż w Asystencie", "在助手中显示", "アシスタントで表示", "Im Assistenten zeigen", "Mostrar en Asistente", "Показати в Асистенті", "Mostrar no Assistente", "Afficher dans l'Assistant", "Показать в Ассистенте"]),
    ("Queued", ["W kolejce", "排队中", "待機中", "In der Warteschlange", "En cola", "У черзі", "Na fila", "En attente", "В очереди"]),
    ("Stopped", ["Zatrzymano", "已停止", "停止しました", "Gestoppt", "Detenida", "Зупинено", "Interrompida", "Arrêtée", "Остановлено"]),
    ("Stop task", ["Zatrzymaj zadanie", "停止任务", "タスクを停止", "Auftrag stoppen", "Detener tarea", "Зупинити завдання", "Parar tarefa", "Arrêter la tâche", "Остановить задачу"]),
    ("Assistant tasks", ["Zadania asystenta", "助手任务", "アシスタントのタスク", "Aufträge des Assistenten", "Tareas del asistente", "Завдання асистента", "Tarefas do assistente", "Tâches de l'assistant", "Задачи ассистента"]),
    ("Assistant tasks: {}", ["Zadania asystenta: {}", "助手任务：{}", "アシスタントのタスク: {}", "Aufträge des Assistenten: {}", "Tareas del asistente: {}", "Завдання асистента: {}", "Tarefas do assistente: {}", "Tâches de l'assistant : {}", "Задачи ассистента: {}"]),
    ("New background task", ["Nowe zadanie w tle", "新的后台任务", "新しいバックグラウンドタスク", "Neuer Auftrag im Hintergrund", "Nueva tarea en segundo plano", "Нове фонове завдання", "Nova tarefa em segundo plano", "Nouvelle tâche en arrière-plan", "Новая фоновая задача"]),
    ("Here · {}", ["Tutaj · {}", "此处 · {}", "ここ · {}", "Hier · {}", "Aquí · {}", "Тут · {}", "Aqui · {}", "Ici · {}", "Здесь · {}"]),
    ("You have it", ["Masz kontrolę", "由你操作", "あなたが操作中", "Du hast übernommen", "Lo tienes tú", "Керуєте ви", "Com você", "C'est à vous", "Управляете вы"]),
    ("All {} live", ["Wszystkie {} na żywo", "全部 {} 个实时", "{} 件すべてをライブで", "Alle {} live", "Los {} en vivo", "Усі {} наживо", "Todos os {} ao vivo", "Les {} en direct", "Все {} вживую"]),
    ("{} live", ["{} na żywo", "{} 个实时", "{} 件をライブで", "{} live", "{} en vivo", "{} наживо", "{} ao vivo", "{} en direct", "{} вживую"]),
    ("Your desk", ["Twoje biurko", "你的桌面", "あなたのデスク", "Dein Desk", "Tu escritorio", "Ваш стіл", "Sua mesa", "Votre bureau", "Ваш стол"]),
    ("Show {}", ["Pokaż {}", "显示 {}", "{} を表示", "{} zeigen", "Mostrar {}", "Показати {}", "Mostrar {}", "Afficher {}", "Показать {}"]),
    ("Let go of {}", ["Zwolnij {}", "释放 {}", "{} を手放す", "{} freigeben", "Soltar {}", "Відпустити {}", "Liberar {}", "Libérer {}", "Отпустить {}"]),
    ("Browser", ["Przeglądarka", "浏览器", "ブラウザ", "Browser", "Navegador", "Браузер", "Navegador", "Navigateur", "Браузер"]),
    ("Live windows", ["Okna na żywo", "实时窗口", "ライブのウインドウ", "Live-Fenster", "Ventanas en vivo", "Вікна наживо", "Janelas ao vivo", "Fenêtres en direct", "Окна вживую"]),
    ("App window", ["Okno aplikacji", "应用窗口", "アプリのウインドウ", "App-Fenster", "Ventana de la app", "Вікно застосунку", "Janela do app", "Fenêtre de l'app", "Окно приложения"]),
    ("Live app window", ["Okno aplikacji na żywo", "实时应用窗口", "ライブのアプリウインドウ", "App-Fenster live", "Ventana de la app en vivo", "Вікно застосунку наживо", "Janela do app ao vivo", "Fenêtre de l'app en direct", "Окно приложения вживую"]),
    ("On your screen", ["Na twoim ekranie", "在你的屏幕上", "あなたの画面に表示中", "Auf deinem Bildschirm", "En tu pantalla", "На вашому екрані", "Na sua tela", "Sur votre écran", "На вашем экране"]),
    ("In the background, off your screen", ["W tle, poza ekranem", "在后台，不在你的屏幕上", "バックグラウンドで、画面の外", "Im Hintergrund, außerhalb deines Bildschirms", "En segundo plano, fuera de tu pantalla", "У фоні, поза вашим екраном", "Em segundo plano, fora da sua tela", "En arrière-plan, hors de votre écran", "В фоне, за пределами экрана"]),
    ("No live picture of this window", ["Brak obrazu na żywo z tego okna", "此窗口没有实时画面", "このウインドウのライブ映像はありません", "Kein Live-Bild von diesem Fenster", "No hay imagen en vivo de esta ventana", "Немає живого зображення цього вікна", "Sem imagem ao vivo desta janela", "Pas d'image en direct de cette fenêtre", "Нет живого изображения этого окна"]),
    ("The window is yours. Hand back to move it out of your way again.", ["Okno jest twoje. Oddaj, aby znów usunąć je z drogi.", "窗口归你了。交还后它会再次移开。", "ウインドウはあなたのものです。返すと再び邪魔にならない場所へ移ります。", "Das Fenster gehört dir. Gib es zurück, dann verschwindet es wieder aus deinem Weg.", "La ventana es tuya. Devuélvela para apartarla de nuevo.", "Вікно ваше. Поверніть, щоб знову прибрати його з дороги.", "A janela é sua. Devolva para tirá-la do caminho de novo.", "La fenêtre est à vous. Rendez la main pour l'écarter à nouveau.", "Окно ваше. Верните его, чтобы оно снова ушло с дороги."]),
    ("The assistant works in this window off your screen. Take over to bring it to you.", ["Asystent pracuje w tym oknie poza ekranem. Przejmij, aby je do siebie przenieść.", "助手正在屏幕外的这个窗口中工作。接管即可把它移到你面前。", "アシスタントは画面の外でこのウインドウを操作しています。引き継ぐと手元に表示されます。", "Der Assistent arbeitet in diesem Fenster außerhalb deines Bildschirms. Übernimm, um es zu dir zu holen.", "El asistente trabaja en esta ventana fuera de tu pantalla. Toma el control para traerla.", "Асистент працює в цьому вікні поза екраном. Перехопіть, щоб перенести його до себе.", "O assistente trabalha nesta janela fora da sua tela. Assuma para trazê-la até você.", "L'assistant travaille dans cette fenêtre hors de votre écran. Prenez la main pour la faire venir.", "Ассистент работает в этом окне за пределами экрана. Перехватите, чтобы перенести его к себе."]),
    ("Live browsers", ["Przeglądarki na żywo", "实时浏览器", "ライブのブラウザ", "Live-Browser", "Navegadores en vivo", "Браузери наживо", "Navegadores ao vivo", "Navigateurs en direct", "Браузеры вживую"]),
    ("Live browser", ["Przeglądarka na żywo", "实时浏览器", "ライブのブラウザ", "Live-Browser", "Navegador en vivo", "Браузер наживо", "Navegador ao vivo", "Navigateur en direct", "Браузер вживую"]),
    ("Main browser", ["Główna przeglądarka", "主浏览器", "メインのブラウザ", "Hauptbrowser", "Navegador principal", "Основний браузер", "Navegador principal", "Navigateur principal", "Основной браузер"]),
    ("Open {} live", ["Otwórz {} na żywo", "实时打开 {}", "{} をライブで開く", "{} live öffnen", "Abrir {} en vivo", "Відкрити {} наживо", "Abrir {} ao vivo", "Ouvrir {} en direct", "Открыть {} вживую"]),
    ("Loading…", ["Wczytywanie…", "正在加载…", "読み込み中…", "Lädt…", "Cargando…", "Завантаження…", "Carregando…", "Chargement…", "Загрузка…"]),
    ("No live picture", ["Brak obrazu na żywo", "没有实时画面", "ライブ画像なし", "Kein Live-Bild", "Sin imagen en vivo", "Немає живого зображення", "Sem imagem ao vivo", "Pas d'image en direct", "Нет живого изображения"]),
    ("No live picture of this browser", ["Brak obrazu na żywo z tej przeglądarki", "此浏览器没有实时画面", "このブラウザのライブ画像はありません", "Von diesem Browser gibt es kein Live-Bild", "No hay imagen en vivo de este navegador", "Цей браузер не має живого зображення", "Não há imagem ao vivo deste navegador", "Pas d'image en direct de ce navigateur", "У этого браузера нет живого изображения"]),
    ("Live browser you're driving", ["Przeglądarka na żywo, którą sterujesz", "你正在操作的实时浏览器", "あなたが操作中のライブのブラウザ", "Live-Browser, den du steuerst", "Navegador en vivo que controlas", "Браузер наживо, яким керуєте ви", "Navegador ao vivo que você controla", "Navigateur en direct que vous pilotez", "Браузер вживую, которым управляете вы"]),
    ("You're driving: click, scroll, and type in the page. The task waits.", ["Sterujesz: klikaj, przewijaj i pisz na stronie. Zadanie czeka.", "由你操作：在页面中点击、滚动和输入。任务会等待。", "あなたが操作中です。ページでクリック、スクロール、入力できます。タスクは待機します。", "Du steuerst: Klicke, scrolle und tippe in der Seite. Der Auftrag wartet.", "Tienes el control: haz clic, desplázate y escribe en la página. La tarea espera.", "Керуєте ви: клацайте, гортайте й друкуйте на сторінці. Завдання чекає.", "Você está no controle: clique, role e digite na página. A tarefa espera.", "C'est vous qui pilotez : cliquez, faites défiler et tapez dans la page. La tâche attend.", "Управляете вы: нажимайте, прокручивайте и печатайте на странице. Задача ждёт."]),
    ("The assistant is working here. Take over to drive it yourself.", ["Asystent tu pracuje. Przejmij, aby sterować samodzielnie.", "助手正在这里工作。接管即可自己操作。", "アシスタントがここで作業中です。引き継ぐと自分で操作できます。", "Der Assistent arbeitet hier. Übernimm, um selbst zu steuern.", "El asistente está trabajando aquí. Toma el control para manejarlo tú.", "Асистент працює тут. Перехопіть, щоб керувати самостійно.", "O assistente está trabalhando aqui. Assuma para controlar você mesmo.", "L'assistant travaille ici. Prenez la main pour piloter vous-même.", "Ассистент работает здесь. Перехватите, чтобы управлять самостоятельно."]),
    ("This task has finished.", ["To zadanie się zakończyło.", "此任务已完成。", "このタスクは終了しました。", "Dieser Auftrag ist beendet.", "Esta tarea ha terminado.", "Це завдання завершено.", "Esta tarefa terminou.", "Cette tâche est terminée.", "Эта задача завершена."]),
    ("Take over", ["Przejmij", "接管", "引き継ぐ", "Übernehmen", "Tomar el control", "Перехопити", "Assumir", "Prendre la main", "Перехватить"]),
    ("Hand back", ["Oddaj", "交还", "返す", "Zurückgeben", "Devolver", "Повернути", "Devolver", "Rendre la main", "Вернуть"]),
    ("Waiting for limit · resumes {}", ["Czeka na limit · wznowi o {}", "等待额度 · {} 恢复", "上限の解除待ち · {} に再開", "Wartet auf das Limit · weiter um {}", "Esperando el límite · se reanuda a las {}", "Чекає на ліміт · продовжить о {}", "Aguardando o limite · retoma às {}", "En attente de la limite · reprise à {}", "Ждёт лимит · продолжит в {}"]),
    ("Hands off: Hex is using your mouse", ["Nie dotykaj: Hex używa twojej myszy", "请勿操作：Hex 正在使用你的鼠标", "操作しないでください：Hex がマウスを使用中", "Finger weg: Hex benutzt deine Maus", "No toques nada: Hex está usando tu ratón", "Не торкайтеся: Hex використовує вашу мишу", "Não mexa: o Hex está usando seu mouse", "Ne touchez à rien : Hex utilise votre souris", "Не трогайте: Hex использует вашу мышь"]),
    ("Esc to stop", ["Esc, aby zatrzymać", "按 Esc 停止", "Esc で停止", "Esc zum Stoppen", "Esc para detener", "Esc, щоб зупинити", "Esc para parar", "Échap pour arrêter", "Esc — остановить"]),
    ("On the web", ["W sieci", "在网页上", "ウェブ上", "Im Web", "En la web", "В інтернеті", "Na web", "Sur le web", "В интернете"]),
    ("Open the task list", ["Otwórz listę zadań", "打开任务列表", "タスク一覧を開く", "Auftragsliste öffnen", "Abrir la lista de tareas", "Відкрити список завдань", "Abrir a lista de tarefas", "Ouvrir la liste des tâches", "Открыть список задач"]),
    ("{} d", ["{} dni", "{} 天", "{} 日", "{} Tg.", "{} d", "{} дн.", "{} d", "{} j", "{} дн."]),
    ("{} jobs wait for the limit", ["Zadania czekające na limit: {}", "{} 个任务在等待额度", "{} 件が上限の解除待ち", "{} Aufträge warten auf das Limit", "{} trabajos esperan el límite", "Завдань чекає на ліміт: {}", "{} trabalhos aguardam o limite", "{} tâches attendent la limite", "Задач ждут лимит: {}"]),
    ("resumes {}", ["wznowi o {}", "{} 恢复", "{} に再開", "weiter um {}", "se reanuda a las {}", "продовжить о {}", "retoma às {}", "reprise à {}", "продолжит в {}"]),
    ("{} helpers", ["Pomocnicy: {}", "{} 个助手", "{} 人のヘルパー", "{} Helfer", "{} ayudantes", "Помічників: {}", "{} ajudantes", "{} assistants", "Помощников: {}"]),
    ("{} more tasks", ["Więcej zadań: {}", "另外 {} 个任务", "ほかに {} 件のタスク", "{} weitere Aufträge", "{} tareas más", "Ще завдань: {}", "Mais {} tarefas", "{} tâches de plus", "Ещё задач: {}"]),
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
    ("Reply", ["Odpowiedz", "回复", "返信", "Antworten", "Responder", "Відповісти", "Responder", "Répondre", "Ответить"]),
    ("Type your answer…", ["Wpisz odpowiedź…", "输入你的回答…", "回答を入力…", "Antwort eingeben…", "Escribe tu respuesta…", "Введіть відповідь…", "Digite sua resposta…", "Saisissez votre réponse…", "Введите ответ…"]),
    ("Send answer", ["Wyślij odpowiedź", "发送回答", "回答を送信", "Antwort senden", "Enviar respuesta", "Надіслати відповідь", "Enviar resposta", "Envoyer la réponse", "Отправить ответ"]),
    ("Return sends · Esc cancels", ["Return wysyła · Esc anuluje", "Return 发送 · Esc 取消", "Return で送信 · Esc で取消", "Return sendet · Esc bricht ab", "Return envía · Esc cancela", "Return надсилає · Esc скасовує", "Return envia · Esc cancela", "Entrée envoie · Échap annule", "Return отправляет · Esc отменяет"]),
    ("Less", ["Mniej", "收起", "閉じる", "Weniger", "Menos", "Менше", "Menos", "Moins", "Меньше"]),
    ("Open folder {}", ["Otwórz folder {}", "打开文件夹 {}", "フォルダ {} を開く", "Ordner {} öffnen", "Abrir la carpeta {}", "Відкрити папку {}", "Abrir a pasta {}", "Ouvrir le dossier {}", "Открыть папку {}"]),
    ("Open file {}", ["Otwórz plik {}", "打开文件 {}", "ファイル {} を開く", "Datei {} öffnen", "Abrir el archivo {}", "Відкрити файл {}", "Abrir o arquivo {}", "Ouvrir le fichier {}", "Открыть файл {}"]),
    ("Open link {}", ["Otwórz link {}", "打开链接 {}", "リンク {} を開く", "Link {} öffnen", "Abrir el enlace {}", "Відкрити посилання {}", "Abrir o link {}", "Ouvrir le lien {}", "Открыть ссылку {}"]),
    ("Recent results: {}", ["Ostatnie wyniki: {}", "最近的结果：{}", "最近の結果: {}", "Letzte Ergebnisse: {}", "Resultados recientes: {}", "Останні результати: {}", "Resultados recentes: {}", "Résultats récents : {}", "Последние результаты: {}"]),
    ("That didn't work. Please try again.", ["To się nie udało. Spróbuj ponownie.", "这没有成功。请再试一次。", "うまくいきませんでした。もう一度試してください。", "Das hat nicht geklappt. Bitte versuch es noch einmal.", "No funcionó. Inténtalo de nuevo.", "Не вдалося. Спробуйте ще раз.", "Não deu certo. Tente de novo.", "Ça n'a pas marché. Réessaie, s'il te plaît.", "Не получилось. Попробуй ещё раз."]),
    ("Fix", ["Napraw", "修复", "修正", "Beheben", "Arreglar", "Виправити", "Corrigir", "Corriger", "Исправить"]),
    ("Your agent isn't signed in.", ["Twój agent nie jest zalogowany.", "你的智能体尚未登录。", "エージェントにサインインしていません。", "Dein Agent ist nicht angemeldet.", "Tu agente no ha iniciado sesión.", "Ваш агент не ввійшов в обліковий запис.", "Seu agente não está conectado.", "Votre agent n'est pas connecté.", "Ваш агент не выполнил вход."]),
    ("Something the assistant needs isn't installed.", ["Brakuje czegoś, czego asystent potrzebuje.", "助手需要的组件尚未安装。", "アシスタントに必要なものがインストールされていません。", "Etwas, das der Assistent braucht, ist nicht installiert.", "Falta instalar algo que el asistente necesita.", "Не встановлено те, що потрібно асистенту.", "Algo de que o assistente precisa não está instalado.", "Il manque un élément dont l'assistant a besoin.", "Не установлено то, что нужно ассистенту."]),
    ("The assistant is missing a permission.", ["Asystentowi brakuje uprawnienia.", "助手缺少一项权限。", "アシスタントに必要な許可がありません。", "Dem Assistenten fehlt eine Freigabe.", "Al asistente le falta un permiso.", "Асистенту бракує дозволу.", "Falta uma permissão ao assistente.", "Il manque une autorisation à l'assistant.", "Ассистенту не хватает разрешения."]),
    ("Your agent hit its usage limit.", ["Twój agent osiągnął limit użycia.", "你的智能体已达到使用上限。", "エージェントが利用上限に達しました。", "Dein Agent hat sein Nutzungslimit erreicht.", "Tu agente alcanzó su límite de uso.", "Ваш агент досяг ліміту використання.", "Seu agente atingiu o limite de uso.", "Votre agent a atteint sa limite d'utilisation.", "Ваш агент достиг лимита использования."]),
    ("That didn't work.", ["To się nie udało.", "没有成功。", "うまくいきませんでした。", "Das hat nicht geklappt.", "Eso no funcionó.", "Не вийшло.", "Não deu certo.", "Ça n'a pas marché.", "Не получилось."]),
    ("Okay, I'll take that into account at the next step.", ["Dobrze, uwzględnię to w następnym kroku.", "好的，我会在下一步考虑这一点。", "わかりました。次のステップで反映します。", "Okay, ich berücksichtige das beim nächsten Schritt.", "De acuerdo, lo tendré en cuenta en el siguiente paso.", "Гаразд, урахую це на наступному кроці.", "Certo, vou levar isso em conta no próximo passo.", "D'accord, j'en tiendrai compte à la prochaine étape.", "Хорошо, учту это на следующем шаге."]),
    ("On it, in the background. I'll plan it and ask what I need.", ["Zajmę się tym w tle. Zaplanuję to i zapytam, czego potrzebuję.", "这就在后台处理。我会先做计划，并问你我还需要什么。", "バックグラウンドで進めます。計画を立てて、必要なことを確認します。", "Mach ich, im Hintergrund. Ich plane das und frage, was ich noch brauche.", "Me pongo con ello en segundo plano. Lo planificaré y te preguntaré lo que necesite.", "Беруся, у фоні. Спланую це й запитаю, що мені потрібно.", "Deixa comigo, em segundo plano. Vou planejar e perguntar o que eu precisar.", "Je m'en occupe en arrière-plan. Je planifie et je vous demande ce qu'il me faut.", "Займусь этим в фоне. Спланирую и спрошу, что мне нужно."]),
    ("Noted, I'll take that in when I start.", ["Zanotowane, uwzględnię to, gdy zacznę.", "记下了，我开始时会一并处理。", "了解しました。始めるときに含めます。", "Notiert, das nehme ich mit dazu, sobald ich anfange.", "Anotado, lo incluiré cuando empiece.", "Записав, урахую це, коли почну.", "Anotado, vou incluir isso quando começar.", "C'est noté, j'en tiendrai compte en commençant.", "Записал, учту это, когда начну."]),
    ("{title}: waiting for your answer", ["{title}: czeka na twoją odpowiedź", "{title}：等待你的回答", "{title}：あなたの返事待ち", "{title}: wartet auf deine Antwort", "{title}: esperando tu respuesta", "{title}: чекає на вашу відповідь", "{title}: aguardando sua resposta", "{title} : attend votre réponse", "{title}: ждёт вашего ответа"]),
    ("No questions are waiting.", ["Żadne pytania nie czekają.", "没有等待回答的问题。", "待っている質問はありません。", "Keine Fragen offen.", "No hay preguntas pendientes.", "Немає запитань, що чекають.", "Nenhuma pergunta aguardando.", "Aucune question en attente.", "Нет ожидающих вопросов."]),
    ("One question is waiting.", ["Czeka jedno pytanie.", "有一个问题在等待。", "質問が1件待っています。", "Eine Frage ist offen.", "Hay una pregunta pendiente.", "Чекає одне запитання.", "Uma pergunta está aguardando.", "Une question est en attente.", "Ожидает один вопрос."]),
    ("{count} questions are waiting.", ["Czekające pytania: {count}.", "有 {count} 个问题在等待。", "質問が{count}件待っています。", "{count} Fragen sind offen.", "Hay {count} preguntas pendientes.", "Запитань, що чекають: {count}.", "{count} perguntas estão aguardando.", "{count} questions sont en attente.", "Ожидающих вопросов: {count}."]),
    ("Okay, later.", ["Dobrze, później.", "好的，稍后再说。", "わかりました、後にします。", "Okay, später.", "De acuerdo, más tarde.", "Гаразд, пізніше.", "Certo, depois.", "D'accord, plus tard.", "Хорошо, позже."]),
    ("{count} jobs are done: {names}.", ["Gotowe zlecenia ({count}): {names}.", "{count} 个任务已完成：{names}。", "{count}件のジョブが完了しました：{names}。", "{count} Aufträge sind fertig: {names}.", "{count} trabajos terminados: {names}.", "Готові завдання ({count}): {names}.", "{count} trabalhos concluídos: {names}.", "{count} tâches sont terminées : {names}.", "Готовые задания ({count}): {names}."]),
    ("{first} and {last}", ["{first} i {last}", "{first}和{last}", "{first}と{last}", "{first} und {last}", "{first} y {last}", "{first} і {last}", "{first} e {last}", "{first} et {last}", "{first} и {last}"]),
    ("{title}: waiting out a usage limit, running for {elapsed}", ["{title}: czeka na koniec limitu użycia, działa od {elapsed}", "{title}：正在等待使用上限解除，已运行 {elapsed}", "{title}：利用上限の解除待ち、{elapsed} 実行中", "{title}: wartet auf das Nutzungslimit, läuft seit {elapsed}", "{title}: esperando a que pase el límite de uso, en marcha desde hace {elapsed}", "{title}: чекає завершення ліміту, працює вже {elapsed}", "{title}: aguardando o limite de uso, em andamento há {elapsed}", "{title} : attend la fin d'une limite d'utilisation, en cours depuis {elapsed}", "{title}: ждёт окончания лимита, работает уже {elapsed}"]),
    ("{title}: paused, you took over", ["{title}: wstrzymane, przejąłeś sterowanie", "{title}：已暂停，由你接管", "{title}：一時停止中、あなたが操作しています", "{title}: pausiert, du hast übernommen", "{title}: en pausa, tomaste el control", "{title}: призупинено, ви взяли керування", "{title}: pausado, você assumiu", "{title} : en pause, vous avez pris la main", "{title}: приостановлено, вы взяли управление"]),
    ("{title}: waiting for a free slot", ["{title}: czeka na wolne miejsce", "{title}：等待空闲名额", "{title}：空き待ち", "{title}: wartet auf einen freien Platz", "{title}: esperando un hueco libre", "{title}: чекає на вільне місце", "{title}: aguardando uma vaga livre", "{title} : attend une place libre", "{title}: ждёт свободного места"]),
    ("{title}: running for {elapsed}", ["{title}: działa od {elapsed}", "{title}：已运行 {elapsed}", "{title}：{elapsed} 実行中", "{title}: läuft seit {elapsed}", "{title}: en marcha desde hace {elapsed}", "{title}: працює вже {elapsed}", "{title}: em andamento há {elapsed}", "{title} : en cours depuis {elapsed}", "{title}: работает уже {elapsed}"]),
    (", {done} of {steps} steps done", [", wykonano {done} z {steps} kroków", "，已完成 {steps} 步中的 {done} 步", "、{steps} ステップ中 {done} 完了", ", {done} von {steps} Schritten erledigt", ", {done} de {steps} pasos hechos", ", виконано {done} з {steps} кроків", ", {done} de {steps} passos concluídos", ", {done} étapes sur {steps} faites", ", выполнено {done} из {steps} шагов"]),
    (", {ended} of {all} helpers done", [", gotowych pomocników: {ended} z {all}", "，{all} 个助手中已完成 {ended} 个", "、ヘルパー {all} 件中 {ended} 件完了", ", {ended} von {all} Helfern fertig", ", {ended} de {all} ayudantes terminados", ", готово помічників: {ended} з {all}", ", {ended} de {all} ajudantes concluídos", ", {ended} assistants sur {all} terminés", ", готово помощников: {ended} из {all}"]),
    (". Still open: {gaps}", [". Jeszcze otwarte: {gaps}", "。尚未完成：{gaps}", "。未完了：{gaps}", ". Noch offen: {gaps}", ". Aún pendiente: {gaps}", ". Ще відкрито: {gaps}", ". Ainda em aberto: {gaps}", ". Encore ouvert : {gaps}", ". Ещё открыто: {gaps}"]),
    ("a minute", ["minutę", "一分钟", "1 分", "einer Minute", "un minuto", "хвилину", "um minuto", "une minute", "минуту"]),
    ("{n} minutes", ["{n} min", "{n} 分钟", "{n} 分", "{n} Minuten", "{n} minutos", "{n} хв", "{n} minutos", "{n} minutes", "{n} мин"]),
    ("{n} hours", ["{n} godz.", "{n} 小时", "{n} 時間", "{n} Stunden", "{n} horas", "{n} год", "{n} horas", "{n} heures", "{n} ч"]),
    ("{n} days", ["{n} dni", "{n} 天", "{n} 日", "{n} Tagen", "{n} días", "{n} дн.", "{n} dias", "{n} jours", "{n} дн."]),
];

#[cfg(test)]
mod tests {
    /// The overlay's sources the main table's check doesn't read.
    const SOURCES: &[&str] = &[
        include_str!("../../rune_ui/assistant_overlay/ask.rs"),
        include_str!("../../rune_ui/assistant_overlay/results.rs"),
        include_str!("../../rune_ui/assistant_overlay/jobs.rs"),
        include_str!("../../rune_ui/assistant_overlay/wall.rs"),
        include_str!("../../rune_ui/assistant_overlay/host.rs"),
    ];

    /// The string literals passed to `tr` and `tr_fill`.
    fn literals(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        for call in ["tr(", "tr_fill("] {
            for (start, _) in source.match_indices(call) {
                let ident = source[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_');
                let Some(rest) = source[start + call.len()..].trim_start().strip_prefix('"') else {
                    continue;
                };
                if ident {
                    continue;
                }
                let mut literal = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => literal.extend(chars.next()),
                        other => literal.push(other),
                    }
                }
                found.push(literal);
            }
        }
        found
    }

    #[test]
    fn every_overlay_string_has_every_language() {
        let table: std::collections::HashMap<_, _> =
            super::super::all_translations().copied().collect();
        let mut checked = 0;
        for source in SOURCES {
            for literal in literals(source) {
                let row = table
                    .get(literal.as_str())
                    .unwrap_or_else(|| panic!("{literal:?} has no translations"));
                assert!(row.iter().all(|t| !t.is_empty()), "{literal:?}");
                checked += 1;
            }
        }
        assert!(checked > 20, "only {checked}");
    }
}

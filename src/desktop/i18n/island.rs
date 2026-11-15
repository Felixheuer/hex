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
    ("Doing now", ["Teraz robię", "正在做", "いま実行中", "Jetzt", "Haciendo ahora", "Зараз роблю", "Fazendo agora", "En cours", "Сейчас делаю"]),
    ("Up next", ["Następnie", "接下来", "次に", "Als Nächstes", "A continuación", "Далі", "Em seguida", "Ensuite", "Дальше"]),
    ("Understood: {}", ["Zrozumiałem: {}", "已理解：{}", "理解：{}", "Verstanden: {}", "Entendido: {}", "Зрозумів: {}", "Entendido: {}", "Compris : {}", "Понял: {}"]),
    ("{} in front", ["{} na wierzchu", "{} 在前台", "{} が前面", "{} im Vordergrund", "{} delante", "{} на передньому плані", "{} na frente", "{} au premier plan", "{} на переднем плане"]),
    ("Listening · {} is in front", ["Słucham · {} jest na wierzchu", "正在聆听 · {} 在前台", "聞き取り中 · {} が前面", "Hört zu · {} im Vordergrund", "Escuchando · {} está delante", "Слухаю · {} на передньому плані", "Ouvindo · {} está na frente", "À l'écoute · {} est au premier plan", "Слушаю · {} на переднем плане"]),
    ("Release {} to send · Esc to cancel", ["Puść {}, aby wysłać · Esc anuluje", "松开 {} 发送 · Esc 取消", "{} を離すと送信 · Esc でキャンセル", "Lass {} los zum Senden · Esc bricht ab", "Suelta {} para enviar · Esc cancela", "Відпустіть {}, щоб надіслати · Esc скасовує", "Solte {} para enviar · Esc cancela", "Relâchez {} pour envoyer · Échap annule", "Отпустите {}, чтобы отправить · Esc отменяет"]),
    ("Hold {} to change it · Esc stops", ["Przytrzymaj {}, aby to zmienić · Esc zatrzymuje", "按住 {} 可更改 · Esc 停止", "{} を長押しで変更 · Esc で停止", "Halte {}, um es zu ändern · Esc stoppt", "Mantén {} para cambiarlo · Esc lo detiene", "Утримуйте {}, щоб змінити · Esc зупиняє", "Segure {} para mudar · Esc para", "Maintenez {} pour le changer · Échap arrête", "Удерживайте {}, чтобы изменить · Esc останавливает"]),
    ("Say what you need", ["Powiedz, czego potrzebujesz", "说出你需要什么", "用件を話してください", "Sag, was du brauchst", "Di lo que necesitas", "Скажіть, що вам потрібно", "Diga do que precisa", "Dites ce qu'il vous faut", "Скажите, что нужно"]),
    // What a task does now, by its tool's kind (`assistant::view::busy`).
    ("Running a command: {}", ["Uruchamia polecenie: {}", "正在运行命令：{}", "コマンドを実行中: {}", "Führt Befehl aus: {}", "Ejecuta un comando: {}", "Виконує команду: {}", "Executando um comando: {}", "Exécute une commande : {}", "Выполняет команду: {}"]),
    ("Running a command", ["Uruchamia polecenie", "正在运行命令", "コマンドを実行中", "Führt Befehl aus", "Ejecuta un comando", "Виконує команду", "Executando um comando", "Exécute une commande", "Выполняет команду"]),
    ("Searching the code: {}", ["Szuka w kodzie: {}", "正在搜索代码：{}", "コードを検索中: {}", "Sucht im Code: {}", "Busca en el código: {}", "Шукає в коді: {}", "Buscando no código: {}", "Cherche dans le code : {}", "Ищет в коде: {}"]),
    ("Searching the code", ["Szuka w kodzie", "正在搜索代码", "コードを検索中", "Sucht im Code", "Busca en el código", "Шукає в коді", "Buscando no código", "Cherche dans le code", "Ищет в коде"]),
    ("Reading {}", ["Czyta {}", "正在读取 {}", "{} を読み込み中", "Liest {}", "Lee {}", "Читає {}", "Lendo {}", "Lit {}", "Читает {}"]),
    ("Reading a file", ["Czyta plik", "正在读取文件", "ファイルを読み込み中", "Liest eine Datei", "Lee un archivo", "Читає файл", "Lendo um arquivo", "Lit un fichier", "Читает файл"]),
    ("Editing {}", ["Edytuje {}", "正在编辑 {}", "{} を編集中", "Bearbeitet {}", "Edita {}", "Редагує {}", "Editando {}", "Modifie {}", "Редактирует {}"]),
    ("Editing a file", ["Edytuje plik", "正在编辑文件", "ファイルを編集中", "Bearbeitet eine Datei", "Edita un archivo", "Редагує файл", "Editando um arquivo", "Modifie un fichier", "Редактирует файл"]),
    ("Searching the web: {}", ["Szuka w sieci: {}", "正在网上搜索：{}", "ウェブを検索中: {}", "Sucht im Web: {}", "Busca en la web: {}", "Шукає в мережі: {}", "Buscando na web: {}", "Cherche sur le web : {}", "Ищет в интернете: {}"]),
    ("Searching the web", ["Szuka w sieci", "正在网上搜索", "ウェブを検索中", "Sucht im Web", "Busca en la web", "Шукає в мережі", "Buscando na web", "Cherche sur le web", "Ищет в интернете"]),
    ("Loading {}", ["Wczytuje {}", "正在加载 {}", "{} を読み込み中", "Lädt {}", "Carga {}", "Завантажує {}", "Carregando {}", "Charge {}", "Загружает {}"]),
    ("Loading a page", ["Wczytuje stronę", "正在加载网页", "ページを読み込み中", "Lädt eine Seite", "Carga una página", "Завантажує сторінку", "Carregando uma página", "Charge une page", "Загружает страницу"]),
    ("Opening the browser: {}", ["Otwiera przeglądarkę: {}", "正在打开浏览器：{}", "ブラウザーを開いています: {}", "Öffnet Browser: {}", "Abre el navegador: {}", "Відкриває браузер: {}", "Abrindo o navegador: {}", "Ouvre le navigateur : {}", "Открывает браузер: {}"]),
    ("Working in the browser", ["Pracuje w przeglądarce", "正在浏览器中操作", "ブラウザーで作業中", "Arbeitet im Browser", "Trabaja en el navegador", "Працює в браузері", "Trabalhando no navegador", "Travaille dans le navigateur", "Работает в браузере"]),
    ("Helper: {}", ["Pomocnik: {}", "助手：{}", "ヘルパー: {}", "Helfer: {}", "Ayudante: {}", "Помічник: {}", "Ajudante: {}", "Assistant : {}", "Помощник: {}"]),
    ("Working with a helper", ["Pracuje z pomocnikiem", "正在与助手协作", "ヘルパーと作業中", "Arbeitet mit einem Helfer", "Trabaja con un ayudante", "Працює з помічником", "Trabalhando com um ajudante", "Travaille avec un assistant", "Работает с помощником"]),
    ("Working in an app", ["Pracuje w aplikacji", "正在应用中操作", "アプリで作業中", "Arbeitet in einer App", "Trabaja en una app", "Працює в застосунку", "Trabalhando em um app", "Travaille dans une app", "Работает в приложении"]),
    ("Updating its plan", ["Aktualizuje plan", "正在更新计划", "計画を更新中", "Aktualisiert den Plan", "Actualiza su plan", "Оновлює план", "Atualizando o plano", "Met à jour son plan", "Обновляет план"]),
    ("Using a tool", ["Używa narzędzia", "正在使用工具", "ツールを使用中", "Nutzt ein Werkzeug", "Usa una herramienta", "Використовує інструмент", "Usando uma ferramenta", "Utilise un outil", "Использует инструмент"]),
    // A job's Terminal mini and its large view (`assistant_overlay::terminal`).
    ("Terminal", ["Terminal", "终端", "ターミナル", "Terminal", "Terminal", "Термінал", "Terminal", "Terminal", "Терминал"]),
    ("Terminal · {}", ["Terminal · {}", "终端 · {}", "ターミナル · {}", "Terminal · {}", "Terminal · {}", "Термінал · {}", "Terminal · {}", "Terminal · {}", "Терминал · {}"]),
    ("edited {}", ["edytowano {}", "已编辑 {}", "{} を編集", "bearbeitet {}", "editado {}", "змінено {}", "editado {}", "modifié {}", "изменён {}"]),
    ("searching {}", ["szuka {}", "搜索 {}", "{} を検索", "sucht {}", "busca {}", "шукає {}", "busca {}", "cherche {}", "ищет {}"]),
    ("reading {}", ["czyta {}", "读取 {}", "{} を読む", "liest {}", "lee {}", "читає {}", "lê {}", "lit {}", "читает {}"]),
];

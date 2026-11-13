//! Projects' strings (`assistant::projects`): the soul's headings, the
//! spoken answers to project commands, the overlay's chip, picker, and
//! offer, and Assistant settings › Projects. One row per string with every
//! language, split into the main table's six columns and the Portuguese,
//! French, and Russian tables, as `island` does.

macro_rules! project_strings {
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
project_strings![
    // The soul's headings.
    ("Goal", ["Cel", "目标", "目標", "Ziel", "Objetivo", "Мета", "Objetivo", "Objectif", "Цель"]),
    ("People", ["Osoby", "人员", "メンバー", "Personen", "Personas", "Люди", "Pessoas", "Personnes", "Люди"]),
    ("Websites", ["Strony internetowe", "网站", "ウェブサイト", "Websites", "Sitios web", "Вебсайти", "Sites", "Sites web", "Сайты"]),
    ("Repositories", ["Repozytoria", "代码仓库", "リポジトリ", "Repositorys", "Repositorios", "Репозиторії", "Repositórios", "Dépôts", "Репозитории"]),
    ("Accounts", ["Konta", "账户", "アカウント", "Konten", "Cuentas", "Облікові записи", "Contas", "Comptes", "Аккаунты"]),
    ("Tone & preferences", ["Ton i preferencje", "语气与偏好", "トーンと好み", "Ton & Vorlieben", "Tono y preferencias", "Тон і вподобання", "Tom e preferências", "Ton et préférences", "Тон и предпочтения"]),
    ("Key files", ["Ważne pliki", "关键文件", "重要なファイル", "Wichtige Dateien", "Archivos clave", "Ключові файли", "Arquivos principais", "Fichiers clés", "Ключевые файлы"]),
    ("Decisions", ["Decyzje", "决定", "決定事項", "Entscheidungen", "Decisiones", "Рішення", "Decisões", "Décisions", "Решения"]),
    ("Recently learned", ["Ostatnio poznane", "最近了解到的", "最近わかったこと", "Zuletzt gelernt", "Aprendido hace poco", "Нещодавно дізнався", "Aprendido recentemente", "Appris récemment", "Недавно узнал"]),
    ("This is the project's soul. Agent-Desk reads it for every task in the project and adds what it learns under “Recently learned”. Edit anything; delete what's wrong.", [
        "To jest dusza projektu. Agent-Desk czyta ją przy każdym zadaniu w projekcie i dopisuje to, czego się dowie, w sekcji „Ostatnio poznane”. Edytuj wszystko; usuń to, co błędne.",
        "这是项目的灵魂。Agent-Desk 在项目中的每个任务都会读取它，并把了解到的内容添加到“最近了解到的”下面。可以随意编辑，删掉不对的内容。",
        "これはプロジェクトのソウルです。Agent-Desk はプロジェクト内のすべてのタスクでこれを読み、わかったことを「最近わかったこと」に追記します。自由に編集し、間違いは削除してください。",
        "Das ist die Seele des Projekts. Agent-Desk liest sie bei jedem Auftrag im Projekt und ergänzt unter „Zuletzt gelernt“, was es erfährt. Bearbeite alles; lösche, was falsch ist.",
        "Esta es el alma del proyecto. Agent-Desk la lee en cada tarea del proyecto y añade lo que aprende en «Aprendido hace poco». Edita lo que quieras; borra lo que esté mal.",
        "Це душа проєкту. Agent-Desk читає її для кожного завдання в проєкті й додає те, що дізнається, у розділ «Нещодавно дізнався». Редагуйте будь-що; видаляйте хибне.",
        "Esta é a alma do projeto. O Agent-Desk a lê em cada tarefa do projeto e adiciona o que aprende em “Aprendido recentemente”. Edite o que quiser; apague o que estiver errado.",
        "Voici l'âme du projet. Agent-Desk la lit pour chaque tâche du projet et ajoute ce qu'il apprend sous « Appris récemment ». Modifiez tout ; supprimez ce qui est faux.",
        "Это душа проекта. Agent-Desk читает её для каждой задачи в проекте и добавляет узнанное в раздел «Недавно узнал». Редактируйте что угодно; удаляйте неверное."]),
    // What Agent-Desk says to a project command.
    ("Project {name} is active now.", ["Projekt {name} jest teraz aktywny.", "现在使用项目 {name}。", "プロジェクト {name} に切り替えました。", "Projekt {name} ist jetzt aktiv.", "El proyecto {name} está activo.", "Проєкт {name} тепер активний.", "O projeto {name} está ativo agora.", "Le projet {name} est maintenant actif.", "Проект {name} теперь активен."]),
    ("There's no project called {name}. Say “new project {name}” to make it.", ["Nie ma projektu {name}. Powiedz „nowy projekt {name}”, aby go utworzyć.", "没有名为 {name} 的项目。说“新建项目 {name}”即可创建。", "{name} というプロジェクトはありません。「新しいプロジェクト {name}」と言うと作成します。", "Es gibt kein Projekt {name}. Sag „neues Projekt {name}“, um es anzulegen.", "No hay ningún proyecto llamado {name}. Di «nuevo proyecto {name}» para crearlo.", "Немає проєкту {name}. Скажіть «новий проєкт {name}», щоб створити його.", "Não existe projeto {name}. Diga “novo projeto {name}” para criá-lo.", "Il n'y a pas de projet {name}. Dites « nouveau projet {name} » pour le créer.", "Проекта {name} нет. Скажите «новый проект {name}», чтобы создать его."]),
    ("New project {name} is ready and active. Its folder is in Documents › Agent-Desk › Projects.", ["Nowy projekt {name} jest gotowy i aktywny. Jego folder jest w Dokumenty › Agent-Desk › Projects.", "新项目 {name} 已创建并启用。文件夹位于 文稿 › Agent-Desk › Projects。", "新しいプロジェクト {name} を作成して有効にしました。フォルダは 書類 › Agent-Desk › Projects にあります。", "Das neue Projekt {name} ist angelegt und aktiv. Sein Ordner liegt in Dokumente › Agent-Desk › Projects.", "El proyecto nuevo {name} está listo y activo. Su carpeta está en Documentos › Agent-Desk › Projects.", "Новий проєкт {name} готовий і активний. Його тека в Документи › Agent-Desk › Projects.", "O novo projeto {name} está pronto e ativo. A pasta fica em Documentos › Agent-Desk › Projects.", "Le nouveau projet {name} est prêt et actif. Son dossier est dans Documents › Agent-Desk › Projects.", "Новый проект {name} создан и активен. Его папка в Документы › Agent-Desk › Projects."]),
    ("There already is a project {name}; it's active now.", ["Projekt {name} już istnieje; jest teraz aktywny.", "项目 {name} 已存在，现在已启用。", "プロジェクト {name} はすでにあります。有効にしました。", "Das Projekt {name} gibt es schon; es ist jetzt aktiv.", "Ya existe un proyecto {name}; ahora está activo.", "Проєкт {name} уже є; тепер він активний.", "Já existe um projeto {name}; agora ele está ativo.", "Le projet {name} existe déjà ; il est maintenant actif.", "Проект {name} уже есть; теперь он активен."]),
    ("I couldn't change the project.", ["Nie udało się zmienić projektu.", "无法切换项目。", "プロジェクトを切り替えられませんでした。", "Ich konnte das Projekt nicht wechseln.", "No pude cambiar de proyecto.", "Не вдалося змінити проєкт.", "Não consegui trocar de projeto.", "Je n'ai pas pu changer de projet.", "Не удалось сменить проект."]),
    ("I couldn't make the project.", ["Nie udało się utworzyć projektu.", "无法创建项目。", "プロジェクトを作成できませんでした。", "Ich konnte das Projekt nicht anlegen.", "No pude crear el proyecto.", "Не вдалося створити проєкт.", "Não consegui criar o projeto.", "Je n'ai pas pu créer le projet.", "Не удалось создать проект."]),
    ("No project now.", ["Teraz bez projektu.", "现在不使用项目。", "プロジェクトなしにしました。", "Jetzt ohne Projekt.", "Ahora sin proyecto.", "Тепер без проєкту.", "Agora sem projeto.", "Plus de projet maintenant.", "Теперь без проекта."]),
    ("You're in project {name}.", ["Jesteś w projekcie {name}.", "当前项目是 {name}。", "現在のプロジェクトは {name} です。", "Du bist im Projekt {name}.", "Estás en el proyecto {name}.", "Ви в проєкті {name}.", "Você está no projeto {name}.", "Vous êtes dans le projet {name}.", "Вы в проекте {name}."]),
    ("No project is active.", ["Żaden projekt nie jest aktywny.", "当前没有启用项目。", "有効なプロジェクトはありません。", "Kein Projekt ist aktiv.", "No hay ningún proyecto activo.", "Жоден проєкт не активний.", "Nenhum projeto está ativo.", "Aucun projet n'est actif.", "Нет активного проекта."]),
    // The overlay: the rail's chip, its picker, a job's tag, the offer.
    ("No project", ["Bez projektu", "无项目", "プロジェクトなし", "Kein Projekt", "Sin proyecto", "Без проєкту", "Sem projeto", "Aucun projet", "Без проекта"]),
    ("Project: {}", ["Projekt: {}", "项目：{}", "プロジェクト：{}", "Projekt: {}", "Proyecto: {}", "Проєкт: {}", "Projeto: {}", "Projet : {}", "Проект: {}"]),
    ("New project…", ["Nowy projekt…", "新建项目…", "新しいプロジェクト…", "Neues Projekt…", "Nuevo proyecto…", "Новий проєкт…", "Novo projeto…", "Nouveau projet…", "Новый проект…"]),
    ("Or say “switch to project …”", ["Albo powiedz „przełącz na projekt …”", "或者说“切换到项目 …”", "または「プロジェクト … に切り替えて」と話す", "Oder sag „wechsle zu Projekt …“", "O di «cambia al proyecto …»", "Або скажіть «перейди до проєкту …»", "Ou diga “mude para o projeto …”", "Ou dites « passe au projet … »", "Или скажите «переключись на проект …»"]),
    ("Projects", ["Projekty", "项目", "プロジェクト", "Projekte", "Proyectos", "Проєкти", "Projetos", "Projets", "Проекты"]),
    ("Name of the project", ["Nazwa projektu", "项目名称", "プロジェクト名", "Name des Projekts", "Nombre del proyecto", "Назва проєкту", "Nome do projeto", "Nom du projet", "Название проекта"]),
    ("Return makes it · Esc cancels", ["Return tworzy · Esc anuluje", "按 Return 创建 · Esc 取消", "Return で作成 · Esc でキャンセル", "Return legt es an · Esc bricht ab", "Return lo crea · Esc cancela", "Return створює · Esc скасовує", "Return cria · Esc cancela", "Entrée le crée · Échap annule", "Return создаёт · Esc отменяет"]),
    ("In project {}?", ["W projekcie {}?", "放到项目 {}？", "プロジェクト {} で？", "Im Projekt {}?", "¿En el proyecto {}?", "У проєкті {}?", "No projeto {}?", "Dans le projet {} ?", "В проекте {}?"]),
    // Assistant settings › Projects.
    ("Leave", ["Opuść", "退出", "解除", "Verlassen", "Salir", "Вийти", "Sair", "Quitter", "Выйти"]),
    ("Edit soul", ["Edytuj duszę", "编辑灵魂", "ソウルを編集", "Seele bearbeiten", "Editar alma", "Редагувати душу", "Editar alma", "Modifier l'âme", "Изменить душу"]),
    ("Archive", ["Archiwizuj", "归档", "アーカイブ", "Archivieren", "Archivar", "Архівувати", "Arquivar", "Archiver", "В архив"]),
    ("Archived", ["Zarchiwizowany", "已归档", "アーカイブ済み", "Archiviert", "Archivado", "В архіві", "Arquivado", "Archivé", "В архиве"]),
    ("Restore", ["Przywróć", "恢复", "元に戻す", "Wiederherstellen", "Restaurar", "Відновити", "Restaurar", "Restaurer", "Восстановить"]),
    ("Create", ["Utwórz", "创建", "作成", "Anlegen", "Crear", "Створити", "Criar", "Créer", "Создать"]),
    ("New project", ["Nowy projekt", "新建项目", "新しいプロジェクト", "Neues Projekt", "Nuevo proyecto", "Новий проєкт", "Novo projeto", "Nouveau projet", "Новый проект"]),
    ("Each gets a folder in Documents › Agent-Desk › Projects and a soul Agent-Desk fills as it works. Without one, everything works as always", [
        "Każdy dostaje folder w Dokumenty › Agent-Desk › Projects i duszę, którą Agent-Desk uzupełnia podczas pracy. Bez projektu wszystko działa jak zawsze",
        "每个项目在 文稿 › Agent-Desk › Projects 中都有一个文件夹和一个灵魂，Agent-Desk 会在工作时补充它。不选项目时一切照旧",
        "それぞれに 書類 › Agent-Desk › Projects のフォルダと、Agent-Desk が作業しながら書き足すソウルができます。プロジェクトなしでもこれまで通り使えます",
        "Jedes bekommt einen Ordner in Dokumente › Agent-Desk › Projects und eine Seele, die Agent-Desk bei der Arbeit füllt. Ohne Projekt funktioniert alles wie immer",
        "Cada uno tiene una carpeta en Documentos › Agent-Desk › Projects y un alma que Agent-Desk completa mientras trabaja. Sin proyecto, todo funciona como siempre",
        "Кожен отримує теку в Документи › Agent-Desk › Projects і душу, яку Agent-Desk доповнює під час роботи. Без проєкту все працює як завжди",
        "Cada um ganha uma pasta em Documentos › Agent-Desk › Projects e uma alma que o Agent-Desk preenche enquanto trabalha. Sem projeto, tudo funciona como sempre",
        "Chacun a un dossier dans Documents › Agent-Desk › Projects et une âme qu'Agent-Desk complète en travaillant. Sans projet, tout fonctionne comme d'habitude",
        "Каждый получает папку в Документы › Agent-Desk › Projects и душу, которую Agent-Desk пополняет в работе. Без проекта всё работает как обычно"]),
];

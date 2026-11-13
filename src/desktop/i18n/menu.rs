//! The menu bar's and the tray's strings (`macos_rune::status_item`,
//! `desktop::tray_menu`): the status line, the project submenu's header,
//! the "Dictate" section, the speech-model submenu, the meeting rows, and
//! the app's own items. One row per string with every language, split into
//! the main table's six columns and the Portuguese, French, and Russian
//! tables, as `projects` does. The setup's Voice step keeps its strings
//! here too: the menu's "Dictate" and the step share their words.

macro_rules! menu_strings {
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
menu_strings![
    // The status line: "Agent-Desk · 2 jobs running · 1 needs you".
    ("1 job running", ["1 zadanie w toku", "1 个任务正在运行", "1 件の仕事が進行中", "1 Auftrag läuft", "1 trabajo en curso", "Виконується 1 завдання", "1 trabalho em andamento", "1 tâche en cours", "Выполняется 1 задача"]),
    ("1 needs you", ["1 czeka na ciebie", "1 个需要你", "1 件の対応が必要", "1 braucht dich", "1 te necesita", "1 потребує вас", "1 precisa de você", "1 vous attend", "1 ждёт вас"]),
    ("{} need you", ["Czekają na ciebie: {}", "{} 个需要你", "{} 件の対応が必要", "{} brauchen dich", "{} te necesitan", "Потребують вас: {}", "{} precisam de você", "{} vous attendent", "Ждут вас: {}"]),
    // The dictation section's header.
    ("Dictate", ["Dyktowanie", "听写", "音声入力", "Diktieren", "Dictar", "Диктування", "Ditar", "Dicter", "Диктовка"]),
    ("Pause Agent-Desk", ["Wstrzymaj Agent-Desk", "暂停 Agent-Desk", "Agent-Desk を一時停止", "Agent-Desk pausieren", "Pausar Agent-Desk", "Призупинити Agent-Desk", "Pausar o Agent-Desk", "Mettre Agent-Desk en pause", "Приостановить Agent-Desk"]),
    ("Resume Agent-Desk", ["Wznów Agent-Desk", "恢复 Agent-Desk", "Agent-Desk を再開", "Agent-Desk fortsetzen", "Reanudar Agent-Desk", "Відновити Agent-Desk", "Retomar o Agent-Desk", "Reprendre Agent-Desk", "Возобновить Agent-Desk"]),
    ("Restore last capture", ["Przywróć ostatni zrzut", "恢复上次截图", "最後のキャプチャを戻す", "Letzten Screenshot zurückholen", "Restaurar la última captura", "Відновити останній знімок", "Restaurar a última captura", "Restaurer la dernière capture", "Восстановить последний снимок"]),
    // The speech-model submenu.
    ("No downloaded models", ["Brak pobranych modeli", "没有已下载的模型", "ダウンロード済みのモデルはありません", "Keine geladenen Modelle", "No hay modelos descargados", "Немає завантажених моделей", "Nenhum modelo baixado", "Aucun modèle téléchargé", "Нет загруженных моделей"]),
    ("Preparing {}…", ["Przygotowywanie: {}…", "正在准备 {}…", "{} を準備中…", "{} wird vorbereitet…", "Preparando {}…", "Підготовка: {}…", "Preparando {}…", "Préparation de {}…", "Подготовка: {}…"]),
    ("Cancel model switch", ["Anuluj zmianę modelu", "取消切换模型", "モデルの切り替えをキャンセル", "Modellwechsel abbrechen", "Cancelar el cambio de modelo", "Скасувати зміну моделі", "Cancelar a troca de modelo", "Annuler le changement de modèle", "Отменить смену модели"]),
    ("Model switch failed — open Settings…", ["Zmiana modelu nie powiodła się — otwórz Ustawienia…", "切换模型失败 — 打开设置…", "モデルを切り替えられませんでした — 設定を開く…", "Modellwechsel fehlgeschlagen – Einstellungen öffnen…", "No se pudo cambiar de modelo: abrir Ajustes…", "Не вдалося змінити модель — відкрити Налаштування…", "A troca de modelo falhou — abrir Ajustes…", "Échec du changement de modèle — ouvrir les Réglages…", "Не удалось сменить модель — открыть настройки…"]),
    ("Manage models…", ["Zarządzaj modelami…", "管理模型…", "モデルを管理…", "Modelle verwalten…", "Gestionar modelos…", "Керувати моделями…", "Gerenciar modelos…", "Gérer les modèles…", "Управление моделями…"]),
    // The meeting rows.
    ("Recording meeting", ["Nagrywanie spotkania", "正在录制会议", "会議を録音中", "Meeting wird aufgenommen", "Grabando la reunión", "Запис зустрічі", "Gravando a reunião", "Enregistrement de la réunion", "Запись встречи"]),
    ("Starting meeting recording…", ["Rozpoczynanie nagrywania spotkania…", "正在开始录制会议…", "会議の録音を開始中…", "Meeting-Aufnahme startet…", "Iniciando la grabación de la reunión…", "Запуск запису зустрічі…", "Iniciando a gravação da reunião…", "Démarrage de l'enregistrement de la réunion…", "Запуск записи встречи…"]),
    ("Transcribing meeting…", ["Transkrybowanie spotkania…", "正在转写会议…", "会議を文字起こし中…", "Meeting wird transkribiert…", "Transcribiendo la reunión…", "Розшифрування зустрічі…", "Transcrevendo a reunião…", "Transcription de la réunion…", "Расшифровка встречи…"]),
    ("Recording · {title} · {time}", ["Nagrywanie · {title} · {time}", "录制中 · {title} · {time}", "録音中 · {title} · {time}", "Aufnahme · {title} · {time}", "Grabando · {title} · {time}", "Запис · {title} · {time}", "Gravando · {title} · {time}", "Enregistrement · {title} · {time}", "Запись · {title} · {time}"]),
    // The app's own items.
    ("Settings…", ["Ustawienia…", "设置…", "設定…", "Einstellungen…", "Ajustes…", "Налаштування…", "Ajustes…", "Réglages…", "Настройки…"]),
    ("Check for updates…", ["Sprawdź aktualizacje…", "检查更新…", "アップデートを確認…", "Nach Updates suchen…", "Buscar actualizaciones…", "Перевірити оновлення…", "Verificar atualizações…", "Rechercher des mises à jour…", "Проверить обновления…"]),
    ("Download update…", ["Pobierz aktualizację…", "下载更新…", "アップデートをダウンロード…", "Update laden…", "Descargar actualización…", "Завантажити оновлення…", "Baixar atualização…", "Télécharger la mise à jour…", "Скачать обновление…"]),
    // The setup's Voice step (`rune_ui::chats::sheet::voice`): the speech
    // model's download and the optional dictation try.
    ("Agent-Desk understands you right on this Mac: its speech model downloads once. Nothing else may take your shortcuts.", [
        "Agent-Desk rozumie cię bezpośrednio na tym Macu: model mowy pobiera się raz. Nic innego nie może przejąć twoich skrótów.",
        "Agent-Desk 直接在这台 Mac 上听懂你：语音模型只需下载一次。其他任何东西都不能占用你的快捷键。",
        "Agent-Desk はこの Mac 上であなたの声を理解します。音声モデルのダウンロードは一度だけです。ショートカットはほかに取られないようにします。",
        "Agent-Desk versteht dich direkt auf diesem Mac: Das Sprachmodell wird einmal geladen. Nichts anderes darf deine Kürzel belegen.",
        "Agent-Desk te entiende en este mismo Mac: el modelo de voz se descarga una sola vez. Nada más puede quedarse con tus atajos.",
        "Agent-Desk розуміє вас просто на цьому Mac: модель мовлення завантажується один раз. Ніщо інше не може зайняти ваші сполучення.",
        "O Agent-Desk entende você neste Mac: o modelo de voz é baixado uma vez. Nada mais pode usar seus atalhos.",
        "Agent-Desk vous comprend directement sur ce Mac : son modèle vocal se télécharge une fois. Rien d'autre ne doit prendre vos raccourcis.",
        "Agent-Desk понимает вас прямо на этом Mac: модель речи загружается один раз. Ничто другое не должно занимать ваши сочетания."]),
    ("Downloading… {}", ["Pobieranie… {}", "正在下载… {}", "ダウンロード中… {}", "Wird geladen… {}", "Descargando… {}", "Завантаження… {}", "Baixando… {}", "Téléchargement… {}", "Загрузка… {}"]),
    ("{} · ready on this Mac", ["{} · gotowy na tym Macu", "{} · 已在这台 Mac 上就绪", "{} · この Mac で準備完了", "{} · bereit auf diesem Mac", "{} · listo en este Mac", "{} · готова на цьому Mac", "{} · pronto neste Mac", "{} · prêt sur ce Mac", "{} · готова на этом Mac"]),
    ("{} · not downloaded yet", ["{} · jeszcze nie pobrany", "{} · 尚未下载", "{} · まだダウンロードされていません", "{} · noch nicht geladen", "{} · aún sin descargar", "{} · ще не завантажена", "{} · ainda não baixado", "{} · pas encore téléchargé", "{} · ещё не загружена"]),
    ("The download stopped. Check the connection and try again", ["Pobieranie zatrzymało się. Sprawdź połączenie i spróbuj ponownie", "下载已停止。请检查网络连接后重试", "ダウンロードが止まりました。接続を確認してもう一度お試しください", "Der Download ist abgebrochen. Prüfe die Verbindung und versuch es noch einmal", "La descarga se detuvo. Revisa la conexión y vuelve a intentarlo", "Завантаження зупинилося. Перевірте з'єднання й спробуйте ще раз", "O download parou. Verifique a conexão e tente de novo", "Le téléchargement s'est arrêté. Vérifiez la connexion et réessayez", "Загрузка остановилась. Проверьте подключение и попробуйте снова"]),
    ("Try dictation", ["Wypróbuj dyktowanie", "试试听写", "音声入力を試す", "Diktieren ausprobieren", "Prueba el dictado", "Спробуйте диктування", "Experimente ditar", "Essayer la dictée", "Попробуйте диктовку"]),
    ("Optional: click the field, hold {} and speak", ["Opcjonalnie: kliknij pole, przytrzymaj {} i mów", "可选：点按输入框，按住 {} 说话", "任意：欄をクリックし、{} を押したまま話します", "Optional: Klick ins Feld, halte {} und sprich", "Opcional: haz clic en el campo, mantén {} y habla", "Необов'язково: клацніть поле, утримуйте {} і говоріть", "Opcional: clique no campo, segure {} e fale", "Facultatif : cliquez dans le champ, maintenez {} et parlez", "Необязательно: щёлкните поле, удерживайте {} и говорите"]),
    ("Once the speech model is here", ["Gdy model mowy będzie gotowy", "语音模型就绪后", "音声モデルの準備ができたら", "Sobald das Sprachmodell da ist", "Cuando el modelo de voz esté listo", "Щойно модель мовлення буде готова", "Assim que o modelo de voz chegar", "Dès que le modèle vocal est là", "Как только модель речи будет готова"]),
    ("Your words land here", ["Twoje słowa pojawią się tutaj", "你的话会出现在这里", "話した言葉がここに入ります", "Deine Worte landen hier", "Tus palabras aparecen aquí", "Ваші слова з'являться тут", "Suas palavras aparecem aqui", "Vos mots arrivent ici", "Ваши слова появятся здесь"]),
    ];

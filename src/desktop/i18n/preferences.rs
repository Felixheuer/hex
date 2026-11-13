//! The Labs rows' translated strings and the History multiple-selection
//! card's, kept beside [`super::TRANSLATIONS`] so the main table stays under
//! the source cap.

/// Columns: Polish, Chinese (Simplified), Japanese, German, Spanish, Ukrainian.
#[rustfmt::skip]
pub(super) const TRANSLATIONS: &[(&str, [&str; 6])] = &[
    ("Live text", ["Tekst na żywo", "实时文字", "ライブテキスト", "Live-Text", "Texto en vivo", "Текст наживо"]),
    ("Live typing", ["Pisanie na żywo", "实时输入", "ライブ入力", "Live-Tippen", "Escritura en vivo", "Живе введення"]),
    ("Open Keyboard Settings", ["Otwórz ustawienia klawiatury", "打开键盘设置", "キーボード設定を開く", "Tastatureinstellungen öffnen", "Abrir ajustes de teclado", "Відкрити налаштування клавіатури"]),
    ("Full screen · Click to capture", ["Cały ekran · Kliknij, aby uchwycić", "全屏 · 点按以截图", "全画面 · クリックで撮影", "Ganzer Bildschirm · Klicken zum Aufnehmen", "Pantalla completa · Haz clic para capturar", "Увесь екран · Клацніть, щоб зняти"]),
    ("Full screen · Click to record", ["Cały ekran · Kliknij, aby nagrać", "全屏 · 点按以录制", "全画面 · クリックで収録", "Ganzer Bildschirm · Klicken zum Aufzeichnen", "Pantalla completa · Haz clic para grabar", "Увесь екран · Клацніть, щоб записати"]),
    ("Screen recording", ["Nagranie ekranu", "屏幕录制", "画面収録", "Bildschirmaufnahme", "Grabación de pantalla", "Запис екрана"]),
    ("Nothing was said", ["Nic nie powiedziano", "没有说话", "何も話されていません", "Nichts gesagt", "No se dijo nada", "Нічого не сказано"]),
    ("Trim", ["Przytnij", "修剪", "トリム", "Kürzen", "Recortar", "Обрізати"]),
    ("Open in Editor", ["Otwórz w edytorze", "在编辑器中打开", "エディタで開く", "Im Editor öffnen", "Abrir en el editor", "Відкрити в редакторі"]),
    ("Show in Finder", ["Pokaż w Finderze", "在访达中显示", "Finder に表示", "Im Finder zeigen", "Mostrar en Finder", "Показати у Finder"]),
    ("Move to Trash", ["Przenieś do kosza", "移到废纸篓", "ゴミ箱に入れる", "In den Papierkorb legen", "Trasladar a la papelera", "Перемістити в смітник"]),
    ("Transform typed text", ["Przekształć wpisany tekst", "转换输入的文本", "入力したテキストを変換", "Eingegebenen Text umwandeln", "Transformar texto escrito", "Перетворити введений текст"]),
    ("Typed text", ["Wpisany tekst", "输入的文本", "入力したテキスト", "Eingegebener Text", "Texto escrito", "Введений текст"]),
    ("Runs Output on what you typed", ["Uruchamia Wyjście na wpisanym tekście", "对输入的文本运行输出步骤", "入力したテキストに出力を実行", "Wendet Ausgabe auf eingegebenen Text an", "Aplica Salida a lo que escribiste", "Застосовує Вивід до введеного тексту"]),
    ("Replaces what you typed where it is", ["Zastępuje wpisany tekst na miejscu", "就地替换已输入的文本", "入力したテキストをその場で置き換えます", "Ersetzt den eingegebenen Text an Ort und Stelle", "Reemplaza lo que escribiste en el mismo lugar", "Замінює введений текст на місці"]),
    ("Terminal paste shortcut", ["Skrót wklejania w terminalu", "终端粘贴快捷键", "端末貼り付けショートカット", "Kürzel für Terminal-Einfügen", "Atajo de pegado en terminal", "Комбінація вставлення в термінал"]),
    ("Release notes", ["Informacje o wydaniu", "发行说明", "リリースノート", "Versionshinweise", "Notas de la versión", "Примітки до випуску"]),
    ("Appearance", ["Wygląd", "外观", "外観", "Darstellung", "Apariencia", "Вигляд"]),
    ("Only on speakers", ["Tylko przez głośniki", "仅扬声器", "スピーカーのみ", "Nur über Lautsprecher", "Solo con altavoces", "Лише через динаміки"]),
    ("Could not play: {}", ["Nie udało się odtworzyć: {}", "无法播放：{}", "再生できませんでした: {}", "Wiedergabe fehlgeschlagen: {}", "No se pudo reproducir: {}", "Не вдалося відтворити: {}"]),
    ("No audio output is available.", ["Brak dostępnego wyjścia audio.", "没有可用的音频输出。", "利用できる音声出力がありません。", "Es ist keine Audioausgabe verfügbar.", "No hay ninguna salida de audio disponible.", "Немає доступного аудіовиходу."]),
    ("Playback stopped.", ["Odtwarzanie zostało przerwane.", "播放已停止。", "再生が停止しました。", "Die Wiedergabe wurde beendet.", "La reproducción se detuvo.", "Відтворення зупинилося."]),
    ("Extra dictation shortcut", ["Dodatkowy skrót dyktowania", "额外的听写快捷键", "追加の音声入力ショートカット", "Weiteres Diktierkürzel", "Atajo de dictado adicional", "Додаткова комбінація для диктування"]),
    ("Extra shortcut {}", ["Dodatkowy skrót {}", "额外快捷键 {}", "追加のショートカット {}", "Weiteres Kürzel {}", "Atajo adicional {}", "Додаткова комбінація {}"]),
    ("Add shortcut", ["Dodaj skrót", "添加快捷键", "ショートカットを追加", "Kürzel hinzufügen", "Añadir atajo", "Додати комбінацію"]),
    ("Switch model with keyboard language", ["Zmieniaj model z językiem klawiatury", "随键盘语言切换模型", "キーボード言語に合わせてモデルを切り替える", "Modell mit Tastatursprache wechseln", "Cambiar modelo con el idioma del teclado", "Перемикати модель разом із мовою клавіатури"]),
    ("{} dictations selected", ["Zaznaczone dyktowania: {}", "已选择 {} 个听写", "{} 件の音声入力を選択中", "{} Diktate ausgewählt", "{} dictados seleccionados", "Вибрано диктувань: {}"]),
    ("Copy puts the selected dictations on the clipboard, oldest first, with a blank line between each", ["Kopiowanie umieszcza zaznaczone dyktowania w schowku, od najstarszego, z pustą linią między każdym", "复制会把选中的听写放到剪贴板，最旧的在前，每条之间空一行", "コピーすると選択した音声入力が古い順にクリップボードへ入り、それぞれの間に空行が入ります", "Kopieren legt die ausgewählten Diktate in die Zwischenablage, das älteste zuerst, mit einer Leerzeile dazwischen", "Copiar pone los dictados seleccionados en el portapapeles, del más antiguo al más reciente, con una línea en blanco entre cada uno", "Копіювання кладе вибрані диктування в буфер обміну, від найстарішого, з порожнім рядком між ними"]),
    ("Delete {} dictations", ["Usuń dyktowania: {}", "删除 {} 个听写", "{} 件の音声入力を削除", "{} Diktate löschen", "Eliminar {} dictados", "Видалити диктування: {}"]),
];

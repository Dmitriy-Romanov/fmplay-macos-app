# Анализ неиспользуемого кода и ресурсов

В результате статического анализа кодовой базы проекта **fmplay-macos-app** были обнаружены следующие неиспользуемые элементы (структуры данных, поля, а также файлы ресурсов).

---

## 1. Неиспользуемые поля в структурах (Rust)

В файле [stations.rs](file:///e:/Antigravity/fmplay-macos-app/src/stations.rs) присутствуют поля, которые декодируются из JSON/API, но не используются в логике приложения или в UI:

### `StationRaw::site`
* **Файл:** [stations.rs:L17](file:///e:/Antigravity/fmplay-macos-app/src/stations.rs#L17)
* **Описание:** Поле `site` в структуре `StationRaw` декодируется из внешнего API, но в функции `parse_stations` оно полностью игнорируется.
* **Примечание:** Ошибки компиляции нет благодаря атрибуту `#[allow(dead_code)]` на уровне структуры `StationRaw`.

### `Station::category`
* **Файл:** [stations.rs:L43](file:///e:/Antigravity/fmplay-macos-app/src/stations.rs#L43)
* **Описание:** Поле `category` заполняется при парсинге и сериализуется в JSON для фронтенда, однако ни в логике на стороне Rust, ни в JS-коде шаблона [app.html](file:///e:/Antigravity/fmplay-macos-app/src/ui/app.html) значение `category` не считывается и не отображается.

### `Station::position`
* **Файл:** [stations.rs:L44](file:///e:/Antigravity/fmplay-macos-app/src/stations.rs#L44)
* **Описание:** Поле `position` парсится из API и сохраняется, но не используется. Сортировка станций в приложении происходит по алфавиту (по полю `name`), а позиция нигде не выводится.

---

## 2. Неиспользуемые файлы ресурсов (Assets)

В папке `assets/` находятся графические файлы, на которые нет ссылок в коде или конфигурационных файлах:

* [assets/fmplay-icon.png](file:///e:/Antigravity/fmplay-macos-app/assets/fmplay-icon.png) — Иконка приложения в формате PNG. В скрипте сборки [build_app.sh](file:///e:/Antigravity/fmplay-macos-app/scripts/build_app.sh#L20) используется исключительно файл `AppIcon.icns`. Этот PNG-файл нигде не задействован.
* [assets/400x400bb-75.webp](file:///e:/Antigravity/fmplay-macos-app/assets/400x400bb-75.webp) — Изображение-заглушка или старая обложка. Код загружает обложки динамически по URL с сервера FMPLAY, а этот локальный файл не импортируется.

> [!NOTE]
> Скриншот `fmplay screenshot.png` в корне проекта не является неиспользуемым, так как на него ссылается [README.md](file:///e:/Antigravity/fmplay-macos-app/README.md#L11).

---

## Рекомендации

1. **Для очистки структуры данных:** 
   Если в будущем категории или сортировка по позиции не планируются, можно удалить поля `category` и `position` из структуры `Station` в [stations.rs](file:///e:/Antigravity/fmplay-macos-app/src/stations.rs).
2. **Для очистки ресурсов:**
   Файлы `assets/fmplay-icon.png` и `assets/400x400bb-75.webp` можно безопасно удалить из репозитория для уменьшения размера исходного кода.

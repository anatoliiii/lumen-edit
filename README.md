# Codium RS

Нативный редактор кода на Rust + egui (wgpu) без Electron и без JS. Быстро открыть
проект локально или на удалённой машине по SSH, править, запускать во встроенном
терминале и работать с git.

## Возможности

- **Редактор**: свой виджет (рисуются только видимые строки, rope-буфер), несколько
  курсоров (Alt+клик, Ctrl+D, Ctrl+Alt+↑/↓), автозакрытие скобок, умные отступы,
  перемещение и дублирование строк, комментирование, поиск и замена (regex),
  подсветка парных скобок и вхождений слова, направляющие отступов.
- **Подсветка синтаксиса** (tree-sitter): Rust, Bash, Python, C, C++, C#, Go, HTML,
  CSS, JavaScript, TypeScript/TSX, JSON, TOML.
- **LSP**: автодополнение, hover, сигнатуры, переход к определению (F12, Ctrl+клик),
  ссылки (Shift+F12), переименование (F2), быстрые исправления (Ctrl+.), форматирование,
  диагностика с подчёркиванием и сообщением в конце строки.
- **SSH**: используется системный OpenSSH (ваш `~/.ssh/config`, ключи, агент,
  ProxyJump, пароли и 2FA вводятся в окне подключения). Одно мастер-соединение
  (ControlMaster) на всё: файлы, терминал, git, языковые серверы запускаются на
  удалённой машине.
- **Агент** (`codium-agent`): маленький статический бинарник, который редактор сам
  загружает на хост (`~/.cache/codium-rs/`). Даёт быстрый поиск и слежение за файлами.
  Без него всё работает через обычные shell-команды.
- **Терминал**: полноценный (vim, htop, mc), несколько вкладок, Ctrl+клик по
  `файл:строка:столбец` открывает место в редакторе.
- **Git**: статус в дереве и отдельная панель, stage/unstage/откат файлов и отдельных
  фрагментов, коммит/amend, push/pull/fetch/sync, ветки (переключение, создание,
  merge, rebase, удаление), stash, история с графом (cherry-pick, revert, reset),
  blame текущей строки, отметки изменений у номеров строк, разрешение конфликтов
  кнопками прямо в редакторе, продолжение/прерывание merge и rebase.
- **Задачи**: автоопределение для Cargo, Go, CMake, Meson, Make, just, Python, .NET,
  package.json и свои задачи в `.codium/tasks.toml`. F5 — запуск, Ctrl+Shift+B — сборка.

## CLI и MCP

Запущенный редактор слушает управляющий сокет `$XDG_RUNTIME_DIR/codium-rs/ctl-<pid>.sock`.
Через него доступны все команды GUI и не только: чтение и правка буферов,
LSP-запросы, диагностика, поиск, git, терминалы, выполнение команд на хосте проекта
(в том числе по SSH), скриншот окна.

```sh
codium-rs src/main.rs:42            # открыть в уже запущенном окне (как `code`)
codium-rs state                     # что открыто, курсор, git, ошибки
codium-rs methods                   # все методы; codium-rs help <метод> — параметры
codium-rs commands                  # id всех команд меню
codium-rs cmd git_sync              # любая команда меню
codium-rs read_file src/lib.rs start_line=1 end_line=50 --line_numbers
codium-rs edit src/lib.rs '[{"start_line":3,"start_column":1,"end_line":3,"end_column":1,"text":"// hi\n"}]' --save
codium-rs hover src/lib.rs 10 5     # definition / references / completion так же
codium-rs diagnostics
codium-rs search 'TODO' --include .rs
codium-rs exec 'cargo test'         # на хосте проекта, код выхода сохраняется
codium-rs terminal_new --command 'cargo run'; codium-rs terminal_read --wait_ms 2000
codium-rs git_commit 'fix: …' --all
codium-rs screenshot path=shot.png
```

Аргументы: `key=value`, `--key value`, `--flag`, JSON или значения по порядку; `--json`
печатает сырой JSON. Несколько окон: `codium-rs instances`, выбор — `CODIUM_SOCKET=…`.

**MCP** (`codium-rs mcp`, stdio) — для Claude Code и других агентов:

```sh
claude mcp add codium -- /путь/к/codium-rs mcp
```

Инструменты сгруппированы по сущностям, чтобы не занимать контекст агента (10
инструментов, ~2,7 тыс. токенов): `state`, `workspace`, `file`, `search`, `code`, `exec`,
`terminal`, `task`, `git`, `command`; операция выбирается параметром `action`
(`file` + `action: "edit"` и т.д.). Если редактор не запущен, MCP-сервер запускает его
сам (отключить: `CODIUM_MCP_NO_LAUNCH=1`).

## Языковые серверы (только нативные, без Node.js)

| Язык | Сервер |
|---|---|
| Rust | `rust-analyzer` |
| C / C++ | `clangd` |
| Go | `gopls` |
| Python | `ty server` + `ruff server` (запасной вариант — `pylsp`) |
| C# | `csharp-ls` (запасной вариант — `OmniSharp -lsp`) |
| TS / JS | `tsgo --lsp --stdio` + `biome lsp-proxy` |
| CSS, JSON | `biome lsp-proxy` |
| HTML | `superhtml lsp` |
| TOML | `taplo lsp stdio` |
| Bash | `shellcheck` (диагностика) |

Сервер должен быть установлен там, где лежит проект (локально или на SSH-хосте).
Отсутствующие просто не запускаются; список виден во вкладке «Вывод». Отключить
сервер можно в настройках, добавить свой — в `~/.config/codium-rs/config.toml`:

```toml
[[lsp]]
name = "zls"
command = "zls"
languages = ["zig"]
```

## Сборка и запуск

```sh
cargo build --release
./scripts/build-agents.sh          # агенты для x86_64 и aarch64 (musl, статические)
./target/release/codium-rs [папка [файл]]
./target/release/codium-rs ssh://host/путь/к/проекту
```

Для агентов нужны цели `rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl`;
C-тулчейн для кросс-сборки не нужен (линкуется встроенным `rust-lld`). Редактор ищет
агентов в `agents/` рядом с бинарником, в `target/agents/`,
`~/.local/share/codium-rs/agents/` и `/usr/lib/codium-rs/agents/`.

На aarch64-машине сам редактор собирается нативно той же командой `cargo build --release`.

## Горячие клавиши

| | |
|---|---|
| Ctrl+P / Ctrl+Shift+P | перейти к файлу / палитра команд |
| Ctrl+G | перейти к строке |
| Ctrl+O / Ctrl+Alt+O | открыть папку / подключиться по SSH |
| Ctrl+Shift+E / F / G / R | проводник / поиск / git / задачи |
| Ctrl+` / Ctrl+Shift+` | терминал / новый терминал |
| F5 / Ctrl+F5 / Shift+F5 | запуск / повторить / остановить |
| Ctrl+Shift+B | сборка |
| Ctrl+Space | автодополнение |
| F12, Shift+F12, F2, Ctrl+. | определение, ссылки, переименование, исправления |
| Ctrl+Shift+I | форматирование |
| Ctrl+D, Alt+клик | следующее вхождение / ещё курсор |
| Alt+↑/↓, Shift+Alt+↑/↓ | переместить / дублировать строки |
| Ctrl+/ | комментарий |
| Ctrl+Enter (в панели git) | коммит |

## Настройки

`~/.config/codium-rs/config.toml` (или окно «Настройки», Ctrl+,): размеры шрифтов, свой
моноширинный шрифт (`editor_font = "/путь/к/шрифту.ttf"`), табуляция, форматирование
при сохранении, автосохранение, blame, агент, `pull --rebase`, языковые серверы.
Открытые файлы запоминаются для каждого проекта.

## Структура

```text
shared/     протокол агента, операции с файлами и поиск (общие для агента и редактора)
agent/      codium-agent — помощник на удалённой машине
src/
  host/     локальный хост и SSH: мастер-соединение, агент, запасной shell-режим
  text.rs   буфер: rope, курсоры, undo/redo, правки
  syntax.rs tree-sitter, подсветка в фоне
  lsp/      LSP-клиент, shellcheck
  terminal.rs  PTY + vt100
  git.rs    git через командную строку
  tasks.rs  задачи
  app/      интерфейс: редактор, проводник, поиск, git, панели, палитра, SSH
            app/control.rs — выполнение CLI/MCP-запросов
  control/  управляющий сокет, реестр методов, MCP-сервер
  cli.rs    командная строка
```

## Разработка

```sh
cargo test                                   # тесты, включая SSH-режим через поддельный ssh
cargo test -- --ignored                      # тест с настоящим rust-analyzer
PATH=$PWD/scripts/fake-ssh-dir:$PATH cargo run -- ssh://fake$PWD   # удалённый режим без sshd
CODIUM_LOG=1 cargo run                       # журнал LSP-обмена
```

Для скриншотов: `CODIUM_SCREENSHOT=out.ppm`, `CODIUM_SCREENSHOT_FRAME=N`,
`CODIUM_DEBUG_VIEW=git,terminal,palette,search=текст,goto=3:5,type=v|.,click=x:y`.

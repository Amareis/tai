# TAI — Terminal Agent Interface

> Rust-бинарник: LLM-агент в цикле тиков. Видит вывод команд, решает, действует.

## Как работает

### Запуск

```bash
tai                          # обычный запуск
tai --tui                   # с выводом логов и стриминга в консоль
tai --debug                 # один тик и выход (debug-режим)
tai --max-ticks 10          # ограничить число тиков
tai --no-delegate           # отключить delegate-блоки
tai ./some-dir              # открыть/создать сессию для указанной директории
```

В проекте уже настроен `.env` с моделью и API-ключом — можно запускать смело, не перепроверяя конфигурацию.

CLI (`src/main.rs`) через clap парсит аргументы, читает `tai.md` как начальный ответ (или `src/session/default_tai.md`), парсит его через `parse_response()` и вызывает `run_server()` из `lib.rs`.

### Цикл тиков

`Server` (`src/core/mod.rs`) крутит `loop`:

1. **`tick_tack`** — вызывает `apply_response` для предыдущего ответа агента, затем `agent.step(&state)`
2. **`apply_response`** — обрабатывает сегменты ответа (в порядке: Close, Ask, Write, Edit, Exec, Delegate, File, Watch):
   - `Close` — удаляет окно из `state.segments` и `state.outputs`
   - `Ask` — задаёт вопрос пользователю в терминале, сохраняет ответ как output
   - `Write` — записывает файл через heredoc, затем показывает его через `file`
   - `Edit` — редактирует файл через `ex`-скрипты (с валидацией строк и без пересечений)
   - `Exec` — выполняет команду один раз, результат кешируется в `outputs`
   - `Delegate` — запускает подпроцесс `tai` для вложенной задачи
   - `File` — показывает файл с нумерацией строк (через `backend.file()`)
   - `Watch` — выполняется каждый тик (просмотр файлов, статусы)
   - `Task` — обрабатывается на этапе парсинга: устанавливает `state.task` и флаг `complete`
3. **`build state`** — `State` собирает: системный промпт + инструкции + сегменты (`Vec<ParsedBlock>`) + outputs (`HashMap<String, CmdOutput>`) + `tick_n` + `task`
4. **`agent.step(&state)`** — вызов LLM (streaming через `async-openai`)
5. **Persistence** — `SessionDir` сохраняет `index.md`, `tick`, `responses/{tick}.toml`, `out/{window}.out`, `steps/{tick}.md`

**Выход:** когда `response.complete == true` (или `state.is_completed`), или достигнут `max_ticks`, или включён `debug`, или получен Ctrl-C.

### Формат взаимодействия с моделью

Модель получает user-сообщения с содержимым окон вида:

```
## [window-title]
L1:line one
L2:line two
exit 0
```

И отвечает текстом с code blocks:

```
Some reasoning prose

```watch:build
cargo build
```

```exec:install
cargo add serde
```

```file:src/main.rs
```

```write:readme.md
<<'TAIDELIM'
# Hello
TAIDELIM
```

```edit:src/main.rs
Exactly L10:old line
<<'TAIDELIM'
fn new() {}
TAIDELIM
```

```ask:user
What should I do next?
```

```old-window:close
```

```task
1. Check build
2. Fix errors
```
```

Парсер (`src/response/mod.rs`) понимает heredoc-и внутри блоков — `<<'TAIDELIM'` защищает содержимое от ложных срабатываний на ` ``` `. `Write` и `Edit` блоки обязаны использовать heredoc. Есть `dashboard`-модификатор (например, `watch.dashboard:tree`).

## Структура проекта

```
src/
├── main.rs              # CLI: tai [--debug] [--tui] [--max-ticks N] [--no-delegate] [path]
├── lib.rs               # create_server(), run_server()
├── types.rs             # BlockMode, ParsedBlock
├── agent/
│   ├── mod.rs           # Agent trait, AgentResponse, NopAgent, MockAgent, TestStep
│   ├── llm.rs           # LlmAgent — async-openai streaming + retry с валидацией
│   └── test_agent.rs    # TestAgent — пошаговая проверка для E2E тестов
├── backend/
│   ├── mod.rs           # Backend trait, CmdOutput
│   └── local.rs         # LocalBackend — bash -c через duct
├── core/
│   └── mod.rs           # Server — tick loop, apply_response, persistence
├── session/
│   ├── mod.rs           # SessionDir — управление сессией и файлами на диске
│   ├── system_prompt.txt
│   ├── instructions.txt
│   └── default_tai.md
├── state/
│   └── mod.rs           # State — сборка данных для промпта
├── response/
│   ├── mod.rs           # parse_response(), serialize_blocks()
│   └── edit_command.rs  # EditCommand, parse_edit_command(), валидация edit
 tests/
└── server_test.rs       # E2E тесты с TestAgent + LocalBackend
```

## Ключевые типы

| Тип | Где | Суть |
|-----|-----|------|
| `BlockMode` | `types.rs` | `Watch` (`view`) / `Close` / `Exec` / `Ask` / `File` / `Edit(Option<EditCommand>)` / `Write` / `Task` / `Delegate` |
| `ParsedBlock` | `types.rs` | `window`, `mode`, `content`, `prose`, `dashboard` — один блок ответа |
| `State` | `state/mod.rs` | `system` + `instructions` + `segments[]` + `outputs` + `tick_n` + `task` + `is_completed` |
| `AgentResponse` | `agent/mod.rs` | `reasoning` + `segments[]` + `task` + `complete` + `heredoc_violations` + `edit_parse_errors` |
| `CmdOutput` | `backend/mod.rs` | `exit_code` + `stdout` |
| `EditCommand` | `response/edit_command.rs` | `start`, `end`, `content`, `start_text`, `end_text` — для `ex`-скриптов |
| `SessionDir` | `session/mod.rs` | Работа с `.session/`: `index.md`, `out/`, `responses/`, `tick`, `steps/` |

## Трейты

- **`Agent`** (`agent/mod.rs`) — `async fn step(&self, state: &State) -> Result<AgentResponse, AgentError>`
  - Реализации: `LlmAgent`, `NopAgent`, `MockAgent`, `TestAgent`
  - Ещё `fn as_any(&self) -> Option<&dyn Any>` (для `TestAgent` в тестах)
- **`Backend`** (`backend/mod.rs`) — `async fn run(&self, title: &str, command: &str) -> CmdOutput`
  - Дефолтный метод: `async fn file(&self, title: &str) -> CmdOutput` (через `awk` с нумерацией)
  - Реализация: `LocalBackend` (bash через duct)

## Зависимости

Основные: `tokio`, `async-openai` (streaming + BYOT), `duct` (shell), `clap` (CLI), `tracing` + `tracing-subscriber`, `serde` + `serde_json`, `toml`, `uuid`, `async-trait`, `futures-util`, `thiserror`, `dotenv`.

Clippy: pedantic, panic/indexing/unwrap/expect — deny. Конфиг в `Cargo.toml` (`[lints.clippy]`) и `clippy.toml` (разрешения для тестов). Просто `cargo clippy`, без флагов.

## Команды для проверки

```bash
cargo test                                   # все тесты (юнит + интеграционные)
cargo clippy --fix --allow-dirty --all-targets  # автофикс линтера (pedantic)
cargo clippy                                 # проверить без фикса
```

## Философия

- **Контекстное окно управляется явно.** Модель решает, что читать и выполнять.
- **Никаких спекуляций.** Результат команды — только на следующем тике.
- **Память один тик.** Только предыдущий ответ доступен агенту.

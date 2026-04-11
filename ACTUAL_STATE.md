# TAI — Состояние кода vs Архитектура/План

## Общая оценка

Проект находится в **Phase 4 (MVP — Tick Cycle)**, частично реализован. Фазы 0-3 завершены. Основной цикл (tick → execute → feedback) работает, но многие архитектурные компоненты ещё не созданы.

---

## Структура проекта — Что есть vs Что планировалось

### Существует в коде

| Файл | Назначение | Статус |
|------|-----------|--------|
| `src/main.rs` | CLI: `tai server --hidden --debug` | ✅ Работает, но нет `tai client` subcommand |
| `src/lib.rs` | Модули + `create_server()` / `run_server()` | ✅ |
| `src/types.rs` | `TickTrigger`, `BlockMode`, `ParsedSegment` | ⚠️ Неполный — нет `Window`, `WindowState`, `Session`, `LaunchOpts`, `TaiCommand` |
| `src/config.rs` | `Config`, `KernelConfig`, `ModelConfig`, etc. | ✅ Полный |
| `src/backend/mod.rs` | `TerminalBackend` trait, `BackendCmd`, `CmdResponse`, `WindowId`, `Terminal`, errors | ✅ Полный |
| `src/backend/kitty.rs` | `KittyBackend` — spawn/connect/launch/send/get/close/list/title | ✅ Полный |
| `src/backend/watch.rs` | `Watcher` — track/remove window IDs | ⚠️ Упрощённый — нет background task, нет WatchEvent channel |
| `src/agent/mod.rs` | `Agent` trait, `AgentResponse`, `AgentError`, `NopAgent`, `MockAgent` | ✅ |
| `src/agent/llm.rs` | `LlmAgent` через async-openai со streaming | ✅ Работает |
| `src/agent/test_agent.rs` | `TestAgent` с пошаговой проверкой | ✅ |
| `src/core/mod.rs` | `Server` — run/tick/execute_blocks/collect_feedback | ⚠️ Монолитный, нет event loop |
| `src/core/utils.rs` | `bind()` (unix socket), `sleep_some_or_forever`, `RmFileOnDrop` | ⚠️ Socket не используется в run() |
| `src/prompt/mod.rs` | `Prompt`, `WindowView`, system prompt, `Prompt::build()` | ⚠️ Нет PromptLayout trait |
| `src/response/mod.rs` | `parse_response()` — парсинг code blocks с heredoc/write поддержкой | ✅ Отличная реализация |
| `config.toml` | Runtime конфигурация | ✅ |
| `clippy.toml` | Разрешения для тестов | ✅ |
| `crates/kitty-rc/` | Kitty RC protocol crate | ✅ |

### Не существует (запланировано в ARCHITECTURE.md)

| Файл/Модуль | Назначение | Приоритет |
|-------------|-----------|-----------|
| `src/session/mod.rs` | Session lifecycle | Phase 4 |
| `src/session/manager.rs` | Vec<Window>, Active→Frozen→Archived | Phase 4/6 |
| `src/session/manifest.rs` | session.json | Phase 10 |
| `src/session/snapshot.rs` | Frozen content save/load | Phase 10 |
| `src/prompt/layout.rs` | PromptLayout trait | Phase 7 |
| `src/prompt/budget.rs` | Token budget | Phase 7 |
| `src/prompt/references.rs` | --help/man page collection | Phase 7 |
| `src/prompt/assembler.rs` | Prompt assembly via PromptLayout | Phase 7 |
| `src/routing/mod.rs` | Clap command parser | Phase 3 (по плану), но не создан |
| `src/routing/parser.rs` | BackendCmd from text | — |
| `src/tui/mod.rs` | Ratatui User Viewport | Phase 9 |
| `src/tui/user_view.rs` | Windows tab, Debug tab | Phase 9 |
| `src/tui/status_bar.rs` | Status bar | Phase 9 |
| `src/kernel/mod.rs` | Event loop (The Tick) + trigger system | Phase 4 |
| `src/core/model_view.rs` | Plain text rendering for Model Channel | — |
| `src/models/l_model.rs` | (exists as `agent/llm.rs`) | ✅ Альтернативный путь |

---

## Ключевые расхождения с архитектурой

### 1. Нет Session / Window lifecycle

**Архитектура**: `Window` объекты с состояниями Active→Frozen→Archived, управляемые через `Session`.

**Код**: `Watcher` хранит только `(WindowId, title, command)`. Нет `Window` struct, нет `Session`, нет lifecycle. `execute_text()` запускает shell, отправляет команду, ждёт `at_prompt`, но не замораживает окна — вместо этого `rerun_watch_windows()` повторно отправляет команды при следующем тике.

### 2. Server.run() использует stdin, не Unix socket

**Архитектура**: `tai server` + `tai client --socket PATH` через Unix Domain Socket. Model Channel — plain text через socket.

**Код**: `Server::run()` читает stdin через `tokio::io::stdin()`. В debug mode ждёт Enter. Unix socket `bind()` существует в `core/utils.rs` но не используется. Нет `tai client` subcommand.

### 3. Нет event loop / trigger system

**Архитектура**: `kernel/mod.rs` с `select!` на watcher events + client input + idle timeout.

**Код**: `Server::run()` — простой loop с `wait_trigger(Some(Duration::from_secs(1)))` который всегда возвращает `IdleTimeout`. Нет `select!`, нет watcher channel, нет `WatchEvent`.

### 4. Нет Session mutation через watcher

**Архитектура**: Watcher — background task, владеет `Arc<Mutex<Session>>`. Poll kitty, создаёт Window, freeze при exit, шлёт `WatchEvent::WindowExited`.

**Код**: Watcher — просто HashMap. Server напрямую mutates через `execute_text()`. Нет WatchEvent, нет mpsc channel.

### 5. Block modes отличаются

**Архитектура**: `text` (send-text), `keys` (send-key), `cmd` (TAI command dispatch).

**Код**: `Text` (launch shell + send command + wait), `Close` (close window), `Write` (write file directly). Нет `keys` и `cmd` режимов. `Write` — практичное дополнение.

### 6. Prompt construction упрощена

**Архитектура**: PromptLayout trait с immutable/ephemeral/system слоями, mind.md, references, token budget.

**Код**: `Prompt::build()` — system prompt (hardcoded) + dashboard (all windows) + focused windows (get-text) + previous response + execution feedback. Нет слоёв, нет budget, нет mind.md.

### 7. Модульная структура отличается

**Архитектура**: `models/`, `kernel/`, `session/`, `routing/`, `tui/`.

**Код**: `agent/` (вместо `models/`), `core/` (вместо разделения на `kernel/` + `core/`), нет `session/`, `routing/`, `tui/`.

### 8. `execute_text` launches shell + sends command

**Архитектура**: `text` mode = `send-text` в stdin существующего окна.

**Код**: `execute_text()` создаёт новое окно с пустым shell (zsh), отправляет команду через `send-text`, ждёт `at_prompt`, трекает через Watcher. Это другая модель — каждое выполнение создаёт новое окно.

### 9. Нет `tai:cmd` dispatch

**Архитектура**: `tai:cmd` blocks парсятся в `BackendCmd` через clap.

**Код**: Нет routing/parser.rs. Block mode `cmd` не реализован. TAI commands (launch/close/focus/summarize) не парсятся из ответа модели.

### 10. LlmAgent использует async-openai напрямую

**Архитектура**: `llm` crate — унифицированный интерфейс.

**Код**: `async-openai` crate с BYOT (bring your own transport) для streaming. Работает, но не через абстракцию.

---

## Фактический рабочий цикл

Текущий `Server::run()` делает следующее:

1. **Hardcoded initial blocks**: Читает `cat TASK.md` и `tree --gitignore` через shell окна.
2. **Close window 1**: Закрывает первое (начальное) окно kitty.
3. **Loop**:
   - `wait_trigger(1s)` — всегда `IdleTimeout` (нет реальных триггеров).
   - `tick()`:
     - `rerun_watch_windows()` — повторно отправляет команды в отслеживаемые окна.
     - `Prompt::build()` — собирает промпт из всех окон kitty.
     - `agent.step()` — вызывает LLM.
     - `execute_blocks()` — выполняет блоки из ответа.
     - `collect_feedback()` — собирает результаты выполнений.
     - Сохраняет `(response, feedback)` как `last_tick`.
4. В debug mode: ждёт Enter между тиками.

---

## Clippy / Lint статус

- `[lints.clippy]` в Cargo.toml: `pedantic = deny`, `panic = deny`, `indexing_slicing = deny`, `unwrap_used = deny`, `expect_used = deny`.
- `clippy.toml` разрешает unwrap/expect/panic/indexing в тестах.
- Зависимости имеют явные версии в Cargo.toml — нарушает правило AGENTS.md "добавлять через `cargo add` без явных версий".

---

## Тесты

| Файл | Что тестирует | Статус |
|------|--------------|--------|
| `tests/kitty_test.rs` | KittyBackend integration (spawn, launch, list, get, send, close, title) | ✅ Требует Kitty |
| `tests/server_test.rs` | E2E tick с TestAgent (tick_empty_session, tick_agent_launches_window) | ⚠️ Второй тест может быть хрупким |
| `tests/types_test.rs` | Config сериализация (roundtrip, load, default) | ✅ |
| `src/response/mod.rs` (unit) | Парсинг code blocks (24 теста) | ✅ |
| `src/backend/watch.rs` (unit) | Watcher track/remove (2 теста) | ⚠️ Минимальный |

---

## Приоритеты для продолжения (из PLAN.md Phase 4)

Необходимо для завершения MVP Tick Cycle:

1. **Watcher background task** — poll kitty, detect exit, freeze windows, send WatchEvent
2. **Server event loop** — `select!` на watcher events + client input
3. **Session / Window lifecycle** — Active→Frozen, хотя бы минимально
4. **`bash -c` wrapping** — для корректного `last_cmd_exit_status`
5. **Doc-комментарии** для новых модулей

---

## Резюме

Код работает и демонстрирует основной цикл: промпт → LLM → выполнить блоки → собрать feedback → повторить. Однако архитектурно значимые части (Session lifecycle, event loop, watcher background task, Model Channel через socket, User Viewport через ratatui, `tai:cmd` dispatch) ещё не реализованы. Текущая реализация — функциональный прототип, а не архитектура из ARCHITECTURE.md.
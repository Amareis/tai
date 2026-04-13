# TAI — Terminal Agent Interface

> Rust-бинарник: LLM-агент в цикле тиков. Видит вывод команд, решает, действует.

## Как работает

### Запуск

```bash
tai server          # обычный запуск
tai server --debug  # пошаговый режим (Enter между тиками)
```

CLI (`src/main.rs`) через clap парсит команду `server`, создаёт набор начальных окон (task, tree) и вызывает `run_server()` из `lib.rs`.

### Цикл тиков

`Server` (`src/core/mod.rs`) крутит `loop`:

1. **apply_pending** — берёт сегменты от предыдущего ответа агента, обновляет список `tracked`-команд
2. **run_tracked** — выполняет команды:
   - `View` — выполняется каждый тик (просмотр файлов, статусы)
   - `Exec` — выполняется один раз, результат кешируется (запись файлов, установка пакетов)
   - `Close` — убирает команду из отслеживаемых
3. **build prompt** — системный промпт + содержимое всех окон + предыдущий ответ агента
4. **agent.step()** — вызов LLM (streaming через async-openai)
5. **parse_response** — разбор ответа на `ParsedSegment` (Block / Prose)
6. Сохранить сегменты как pending для следующего тика

**Выход:** когда `tracked` пуст (все окна закрыты).

### Формат взаимодействия с моделью

Модель получает user-сообщения с содержимым окон вида:

```
## [window-title]
exit 0
```
output here
```
```

И отвечает текстом с code blocks:

```
Some reasoning prose

```window-title
command to run
```

```another-title:exec
one-shot command
```

```old-window:close
```
```

Парсер (`src/response/mod.rs`) понимает heredoc-и внутри блоков — `<<'EOF'` защищает содержимое от ложных срабатываний на ` ``` `.

## Структура проекта

```
src/
├── main.rs              # CLI: tai server [--debug]
├── lib.rs               # create_server(), run_server()
├── types.rs             # BlockMode, ParsedSegment
├── agent/
│   ├── mod.rs           # Agent trait, AgentResponse, NopAgent, MockAgent
│   ├── llm.rs           # LlmAgent — async-openai streaming (поддержка reasoning_content)
│   └── test_agent.rs    # TestAgent — пошаговая проверка для E2E тестов
├── backend/
│   ├── mod.rs           # Backend trait, CmdOutput
│   └── local.rs         # LocalBackend — bash -c через duct
├── core/
│   └── mod.rs           # Server — tick loop, tracked commands, apply/execute
├── prompt/
│   ├── mod.rs           # Prompt, TrackedView
│   └── system_prompt.txt # Системный промпт для модели
├── response/
│   └── mod.rs           # parse_response() — парсинг code blocks с поддержкой heredoc
└── tests/
    └── server_test.rs   # E2E тесты с TestAgent + LocalBackend
```

## Ключевые типы

| Тип | Где | Суть |
|-----|-----|------|
| `BlockMode` | `types.rs` | `View` / `Exec` / `Close` — режим окна |
| `ParsedSegment` | `types.rs` | `Block { window, mode, content }` или `Prose(String)` |
| `TrackedCmd` | `core/mod.rs` | Внутренний: title, command, rerun, cached_output/exit |
| `TrackedView` | `prompt/mod.rs` | title, output, exit_code — для сборки промпта |
| `Prompt` | `prompt/mod.rs` | system + tracked[] + previous_response |
| `AgentResponse` | `agent/mod.rs` | reasoning + segments[] |
| `CmdOutput` | `backend/mod.rs` | exit_code + stdout |

## Трейты

- **`Agent`** (`agent/mod.rs`) — `async fn step(&self, prompt: &Prompt) -> Result<AgentResponse, AgentError>`
  - Реализации: `LlmAgent`, `NopAgent`, `MockAgent`, `TestAgent`
- **`Backend`** (`backend/mod.rs`) — `async fn run(&self, title: &str, command: &str) -> CmdOutput`
  - Реализация: `LocalBackend` (bash через duct)

## Зависимости

Основные: `tokio`, `async-openai` (streaming + BYOT), `duct` (shell), `clap` (CLI), `tracing`, `serde`.

Clippy: pedantic, panic/indexing/unwrap — deny.

## Философия

- **Контекстное окно управляется явно.** Модель решает, что читать и выполнять.
- **Никаких спекуляций.** Результат команды — только на следующем тике.
- **Память один тик.** Только предыдущий ответ доступен агенту.

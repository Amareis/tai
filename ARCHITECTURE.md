# TAI — Архитектура

> Техническая архитектура, модель данных, структура кода.

## Общая схема

```
┌─────────────────────────────────────────────────────────────┐
│                        KERNEL (Rust)                        │
│  event loop · prompt assembly · token budget · TUI (ratatui)│
├───────────────┬─────────────────────────────────────────────┤
│  Terminal     │  Session state                               │
│  Backend      │  (Vec<Window>, manifest, snapshots)         │
│  (trait)      │                                              │
├───────────────┴─────────────────────────────────────────────┤
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐       │
│  │ Terminal │ │ Terminal │ │ Terminal │ │ Terminal │       │
│  │ Session  │ │ Session  │ │ Session  │ │ Session  │       │
│  │ bash     │ │ build    │ │ python   │ │ chat     │       │
│  │ (PTY)    │ │ (PTY)    │ │ (PTY)    │ │ (PTY)    │       │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘       │
├─────────────────────────────────────────────────────────────┤
│  Backend impls: KittyBackend, TmuxBackend (future)         │
├─────────────────────────────────────────────────────────────┤
│  TUI: Chat | Windows | Debug (встроен в ядро, ratatui)     │
└─────────────────────────────────────────────────────────────┘
```

---

## Terminal Backend

Модель **не вызывает** терминальные команды напрямую (kitten, tmux). Она пишет команды ядра TAI через `tai:cmd` code blocks. Ядро парсит, валидирует, мапит на конкретный backend.

Зачем:
- Модель может забыть `--hold` → окно пропадёт, вывод потерян
- Модель не знает путь к сокету → команда упадёт
- Хотим поменять Kitty на tmux → не переписываем промпты
- Хотим remote → модель не должна знать про SSH

```rust
trait TerminalBackend {
    async fn launch(&self, opts: LaunchOpts) -> Result<WindowId>;
    async fn send_text(&self, window: &WindowId, text: &str) -> Result<()>;
    async fn send_keys(&self, window: &WindowId, keys: &str) -> Result<()>;
    async fn get_text(&self, window: &WindowId) -> Result<String>;
    async fn close(&self, window: &WindowId) -> Result<()>;
    async fn list_windows(&self) -> Result<Vec<WindowInfo>>;
    async fn set_title(&self, window: &WindowId, title: &str) -> Result<()>;
}
```

Реализации: `KittyBackend` (kitty-rc), `TmuxBackend` (future), `RemoteBackend` (future).

Ядро **гарантирует** `--hold` на всех launch. Модель не знает про kitten, сокеты, SSH.

---

## TUI — интерфейс человека

Ядро предоставляет **встроенный TUI** (ratatui). Отдельные CLI бинарники не нужны.

### Таб Chat
Диалог с моделью. Человек пишет, видит ответы. Те же данные рендерятся как observation для модели.

### Таб Windows
Список окон (dashboard). Клик → focus/summarize/view frozen content.

### Таб Debug
Пошаговое исполнение команд модели. Видишь каждый блок, можешь:
- **Step** — выполнить следующий блок
- **Run All** — выполнить все оставшиеся
- **Edit** — изменить блок перед выполнением
- **Skip** — пропустить блок

### Два представления чата
- Человек видит в TUI: свой perspective (ввод, ответы модели, prose)
- Модель видит в промпте: `## Окно [chat] (focused)` с историей как observation

Одни данные, разный рендер. Ядро рисует TUI для человека, собирает observations для модели.

---

## Формат ответа модели

Модель отвечает текстом с markdown code blocks. **Window-id и mode — обязательны, валидируются при парсинге.**

````markdown
Проверю что билд чистый. Ожидаю exit 0.

```build:text
cargo build 2>&1
```

Запущу тесты в новом окне:

```tai:cmd
launch --title tests -- bash -c "cargo test 2>&1"
```
````

| Режим | Механизм | Для кого |
|---|---|---|
| `text` | `send-text` | программы с stdin (bash, REPL) |
| `keys` | `send-key` | TUI (vim, less, htop) |
| `cmd` | dispatch TAI command | управление окнами |

**Блоки выполняются параллельно.** Sequential — future work.

**Текст перед блоками = ожидания модели.** На следующем тике модель сопоставляет с реальностью. Критично для `keys` где нет exit code.

**Текст вне блоков** — prose в чат.

---

## Слои контекста (Prompt Construction)

Контекст собирается через `PromptLayout` trait.

### А. Immutable Layer — Всегда в начале
- **System Prompt**: Роль, формат ответа, правила, текущий write-target
- **Mind** (`mind.md`): Цели, статус, последние размышления модели
- **References**: `--help` / man page для окон с правом записи

### Б. Ephemeral Layer — Управляемое внимание
- **Dashboard**: Список всех окон с ID, заголовками, статусами
- **Focused Windows**: Полный текст (get-text) только развёрнутых окон
- **Frozen Summaries**: Сводки завершённых окон (с exit code)

### В. System Layer — Всегда в конце
```
[STATUS] Tokens: 15k/128k (12%) | Active: 2 | Frozen: 3 | Write: bash
```

### Лейаут сообщений

Предыдущий ответ модели (assistant) ставится **прямо перед** текущими наблюдениями (user messages). Модель видит свои ожидания и реальность рядом.

Лейаут вынесен в trait для экспериментирования без касания ядра.

---

## Жизненный цикл окон

```
Active → Frozen → Archived

Active:  PTY-процесс. Живой вывод. В промпте если focused.
Frozen:  Снимок завершённого. В RAM + exit code.
Archived: Файл на диске. Не в контексте.
```

### Обнаружение завершения: `--hold` + `at_prompt`

Все окна запускаются с `--hold` (ядро гарантирует). Poll каждые 500мс. Когда `at_prompt == true`:
1. `get-text` — захватить вывод
2. Window → Frozen (RAM + диск)
3. `close` — закрыть окно

### Результаты выполнения

Модель видит вывод **в самих окнах** на следующем тике. Для frozen добавляем exit code. Для keys — модель проверяет себя по содержимому окна.

### Thinking/reasoning

Thinking → **mind.md** отдельной секцией. Debug log для человека, continuity для модели.

---

## Триггеры тика

```
1. at_prompt на tracked окне → команда завершилась → тик
2. Пользователь написал в TUI chat → немедленный тик
3. Idle timeout (default 60s) → статусный тик

Множественные триггеры → один тик
```

---

## Рабочий цикл (The Tick)

```
1. ASSEMBLE   → layout.build(): system + mind + references + предыдущий ответ
                + observations (окна) + status bar
2. INVOKE     → отправить в L-модель
3. RECEIVE    → текст + code blocks + thinking
4. PARSE      → parse_blocks(): window:mode + tai:cmd
5. EXECUTE    → параллельно:
                  tai:cmd → dispatch (launch/close/focus/summarize)
                  text → send-text через backend
                  keys → send-key через backend
6. THINKING   → извлечь thinking → mind.md
7. WAIT       → poll (500мс), ждать триггер:
                  - at_prompt → freeze → тик
                  - TUI chat input → тик
                  - idle timeout (60s) → статусный тик
8. LOOP       → GOTO 1
```

---

## Структура проекта

```
tai/
├── Cargo.toml
├── AGENTS.md
├── ARCHITECTURE.md
├── PLAN.md
├── config.toml
│
├── src/
│   ├── main.rs                 # tai run [--hidden]
│   ├── lib.rs
│   ├── types.rs                # Window, WindowState, Session, LaunchOpts, TaiCommand
│   │
│   ├── backend/                # TerminalBackend trait + реализации
│   │   ├── mod.rs              # trait TerminalBackend
│   │   └── kitty.rs            # KittyBackend (kitty-rc)
│   │
│   ├── session/
│   │   ├── mod.rs
│   │   ├── manager.rs          # Vec<Window>, Active→Frozen→Archived
│   │   ├── manifest.rs         # session.json
│   │   └── snapshot.rs         # frozen save/load
│   │
│   ├── prompt/
│   │   ├── mod.rs
│   │   ├── assembler.rs        # сборка через PromptLayout
│   │   ├── budget.rs           # токены, бюджеты слоёв
│   │   ├── references.rs       # --help / man page
│   │   └── layout.rs           # PromptLayout trait
│   │
│   ├── routing/
│   │   ├── mod.rs
│   │   ├── parser.rs           # parse_blocks(): window:mode + tai:cmd
│   │   └── tai_command.rs      # парсер команд ядра
│   │
│   ├── tui/                    # TUI для человека (ratatui)
│   │   ├── mod.rs              # App state, event loop
│   │   ├── chat.rs             # таб чата
│   │   ├── windows.rs          # таб списка окон
│   │   ├── debug.rs            # таб пошагового исполнения
│   │   └── status_bar.rs       # status bar
│   │
│   ├── kernel/
│   │   └── mod.rs              # event loop (The Tick) + trigger system
│   │
│   └── models/
│       └── l_model.rs          # LLM клиент через llm crate
│
├── sessions/                   # runtime data (gitignored)
│   └── default/
│       ├── session.json
│       ├── mind.md
│       └── history/
│
└── tests/
    └── integration/
        ├── backend_test.rs
        ├── session_test.rs
        ├── routing_test.rs
        └── layout_test.rs
```

---

## Модель данных

```rust
enum WindowState {
    Active { backend_id: String, pid: u32, title: String },
    Frozen { content: String, exit_code: i32, captured_at: DateTime<Utc> },
    Archived { file_path: PathBuf, exit_code: i32 },
}

struct Window {
    id: String,
    state: WindowState,
    focused: bool,
    summary: Option<String>,
    tags: Vec<String>,
}

struct Session {
    id: String,
    windows: Vec<Window>,
    mind_path: PathBuf,
}

enum ParsedSegment {
    Prose(String),
    Block { target: String, mode: BlockMode, content: String },
    TaiCommand(TaiCommand),
    Invalid { raw_header: String, error: String },
}

enum BlockMode { Text, Keys }

enum TaiCommand {
    Launch { title: String, command: String, shell: Option<String> },
    Close { window_id: String },
    Focus { window_id: String },
    Summarize { window_id: String },
}

enum TickTrigger {
    WindowExited { window_id: String, exit_code: i32 },
    UserMessage,
    IdleTimeout,
}

struct LaunchOpts {
    title: String,
    command: String,
    shell: Option<String>,
    hold: bool,  // всегда true
}
```

---

## Зависимости

Добавлять через `cargo add` **без явных версий** — пусть cargo сам подтянет актуальные.

```bash
cargo add kitty-rc ratatui crossterm tokio --features tokio/full
cargo add clap --features clap/derive
cargo add serde --features serde/derive
cargo add serde_json toml
cargo add chrono --features chrono/serde
cargo add uuid --features uuid/v4
cargo add tiktoken-rs llm
cargo add tracing tracing-subscriber thiserror regex
```

Справочно: `llm` crate — унифицированный интерфейс для LLM бэкендов (Claude, GPT, etc). Заменяет ручной HTTP-вызов через reqwest.

---

## Термины

| Термин | Значение |
|--------|----------|
| L-Model | Большая модель (Claude, GPT) |
| S-Model | Лёгкая модель-наблюдатель (отложено) |
| Terminal Session | Реальная PTY-сессия |
| Window | Объект в памяти ядра — Active/Frozen/Archived |
| Mind | Файл `mind.md` — память + thinking |
| Dashboard | Список окон с состояниями |
| Tick | Один цикл assemble → invoke → parse → execute → wait |
| Focus | Окно развёрнуто в промпте |
| Summarize | Окно свёрнуто |
| Frozen | Снимок завершённого процесса |
| Write Target | Окно куда пишется ввод |
| Block | ` ```<window-id>:<mode> ` — обязательные параметры |
| Prose | Текст вне блоков → чат |
| Observation | Содержимое окна как user message |
| Trigger | at_prompt / user message / idle timeout |
| Backend | TerminalBackend trait |
| TAI Command | Команда ядра (launch/close/focus) |
| TUI | Терминальный UI для человека (ratatui) |

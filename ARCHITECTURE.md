# TAI — Архитектура

> Техническая архитектура, модель данных, структура кода.

## Общая схема

```
┌──────────────────────────── KERNEL (Rust) ────────────────────────────┐
│  event loop · prompt assembly · token budget                          │
│  Arc<RwLock<WorldState>> — единое состояние                           │
├────────────────────────────────────────────────────────────────────────┤
│                                                                        │
│  ┌─── User Viewport ──────┐    ┌─── Model Channel ────────────────┐   │
│  │  Терминал человека      │    │  Unix socket → Kitty окно        │   │
│  │  ratatui (dashboard):   │    │  Plain text (НЕ ratatui):        │   │
│  │    Windows              │    │    State header                  │   │
│  │    Debug                │    │    Command log / chat             │   │
│  │    Status bar           │    │    Input prompt                  │   │
│  └─────────────────────────┘    └──────────────────────────────────┘   │
│                                                                        │
│  ┌─── Terminal Backend (trait) ────────────────────────────────────┐    │
│  │  KittyBackend → Kitty RC protocol                               │    │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐          │    │
│  │  │ bash     │ │ build    │ │ python   │ │ vim      │          │    │
│  │  │ (PTY)    │ │ (PTY)    │ │ (PTY)    │ │ (PTY)    │          │    │
│  │  └──────────┘ └──────────┘ └──────────┘ └──────────┘          │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│                                                                        │
│  Backend impls: KittyBackend, TmuxBackend (future)                     │
└────────────────────────────────────────────────────────────────────────┘

Запуск:
  $ tai server                    # в терминале человека
  → spawn Kitty: kitty -- tai client --socket /tmp/tai.sock
  → tai client подключается к Unix сокету
  → server пишет текст в сокет → client печатает в stdout Kitty
  → client читает stdin Kitty → отправляет строку в сокет → server получает
```

### Server-Client: Socket + Plain Text

Model Viewport **не использует ratatui**. Plain text через Unix Domain Socket.

Почему не ratatui:
- ratatui включает alternate screen buffer → get-text не видит scrollback
- Модели нужен **весь** вывод, не только visible viewport
- scrollable widget прячет строки → модель не увидит ушедшее вверх
- ANSI escape soup от ratatui — шум для модели, не сигнал

Как вместо этого:
- Server пишет plain text в Unix socket — просто строки, без форматирования под ширину
- `tai client` читает socket → `write_all()` в stdout → Kitty рендерит
- `tai client` читает stdin → отправляет строки в socket → server получает
- Старый вывод уходит в scrollback, доступен через get-text
- Ресайз не нужен — plain text не зависит от размеров терминала

Модель видит Model Viewport через стандартный механизм: get-text через
Kitty RC (как и для всех окон). Единый путь для всех окон.

Состояние рисуется как:
```
═══════════════════════════════════════════
STATE: Active 3 | Frozen 1 | Tokens: 12k
FOCUS: build (bash, pid 1234)
═══════════════════════════════════════════

[build] $ cargo build 2>&1
   Compiling tai v0.1.0
   Finished dev [unoptimized] | 2.12s
   exit code: 0

> что дальше?
```

Обновление state: новые строки дописываются. Старый контекст в scrollback, не потерян.
Никакого clear screen — просто поток строк.

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

## Два интерфейса

Ядро предоставляет два интерфейса из одного WorldState.

### User Viewport (терминал человека)
**ratatui** на stdout процесса `tai server`. Полноэкранный dashboard для наблюдения.

- **Таб Windows**: Список окон. Клик → focus/summarize/view frozen content.
- **Таб Debug**: Пошаговое исполнение команд модели. Step / Run All / Edit / Skip.
- **Status bar**: Токены, активные окна, write target.

Один ratatui `Terminal<CrosstermBackend<Stdout>>`. Стандартная отрисовка.

### Model Channel (Kitty окно)
**Plain text** через Unix socket. НЕ ratatui. Обычный shell-режим.

**Единственный интерфейс для модели и человека как равноправных пользователей.**

- **State header**: статус системы (окна, токены, focus)
- **Command log**: вывод команд, ответы модели, результаты
- **Input prompt**: `> ` — и модель (через send-text), и человек (через stdin) пишут сюда

Протокол: newline-delimited текст через Unix socket.
- Server → Client: строки для вывода (plain text, без форматирования под ширину)
- Client → Server: строки ввода (то что человек набрал и нажал Enter)
- Ресайз не нужен — plain text не зависит от размеров терминала

### Два представления чата
- Человек видит в Model Channel: свой perspective (ввод, ответы модели, prose)
- Модель видит в промпте: `## Окно [chat] (focused)` с историей как observation

Одни данные, разный рендер. User Viewport — ratatui dashboard. Model Channel — plain text через socket.

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
2. Пользователь написал в Model Viewport (Kitty окно) → немедленный тик
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
│   ├── main.rs                 # tai server | tai client (clap subcommands)
│   ├── lib.rs
│   ├── types.rs                # Window, WindowState, Session, LaunchOpts, TaiCommand
│   │
│   ├── backend/                # TerminalBackend trait + реализации
│   │   ├── mod.rs              # trait TerminalBackend
│   │   ├── kitty.rs            # KittyBackend (kitty-rc)
│   │   └── watch.rs            # ProcessWatch: poll at_prompt
│   │
│   ├── client/                 # Unix socket server/client для Model Channel
│   │   ├── mod.rs              # Socket server (в kernel), socket client (~30 строк)
│   │   └── model_view.rs       # Потоковый текстовый вывод: строки в socket
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
│   ├── tui/                    # ratatui User Viewport (один terminal)
│   │   ├── mod.rs              # Terminal setup, event loop
│   │   ├── user_view.rs        # Windows tab, Debug tab, Status bar
│   │   └── status_bar.rs       # общие компоненты
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

Doc-комментарии и определения типов — в исходниках:

- [`src/types.rs`](src/types.rs) — `WindowState`, `Window`, `Session`, `LaunchOpts`, `BlockMode`, `TaiCommand`, `ParsedSegment`, `TickTrigger`
- [`src/backend/mod.rs`](src/backend/mod.rs) — `TerminalBackend` trait, `WindowId`, `WindowInfo`, `BackendError`
- [`src/config.rs`](src/config.rs) — `Config`, `KernelConfig`, `ModelConfig`, `SessionConfig`, `BackendConfig`
- [`src/client/mod.rs`](src/client/mod.rs) — Unix socket server/client для Model Channel
- [`src/client/model_view.rs`](src/client/model_view.rs) — plain text рендер Model Channel

---

## Режимы работы бинарника

```
tai server [--socket PATH] [--hidden]
  → Инициализирует User Viewport (ratatui) на stdout
  → Создаёт Unix сокет (default: /tmp/tai.sock)
  → spawn Kitty: kitty -- tai client --socket PATH
  → Ждёт подключения client по сокету
  → Пишет plain text в сокет → client печатает в Kitty stdout
  → Читает строки из сокета ← client пересылает stdin Kitty

tai client --socket PATH
  → Подключается к Unix сокету
  → rustyline для ввода (история, навигация по словам, ↑↓)
  → Строки ввода → сокет → server
  → Сокет → stdout → человек видит ответы
  → ~50 строк кода, readline↔socket proxy
```

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
| User Viewport | Терминал человека — ratatui dashboard + debug |
| Model Channel | Kitty окно — plain text через Unix socket, чат + команды |
| tai client | readline↔socket proxy (~50 строк, rustyline: история, навигация по словам) |

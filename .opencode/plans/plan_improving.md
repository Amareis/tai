# План улучшений TAI — решение проблем агента

## Контекст

Анализ проведён на основе изучения всего кода TAI (src/, тесты, конфиг) и
личного опыта работы в качестве агента. Выявлены три критические проблемы,
которые серьёзно снижают надёжность агента, и предложены решения в рамках
текущей архитектуры.

---

## Проблема 1: Агент закрывает или перезаписывает критические окна

### Описание

Агент может отправить `close` для окна `task` или отправить туда другую команду,
в результате чего теряет понимание что ему нужно делать. Это критический сбой —
потеря задачи означает потерю контекста операции.

### Почему происходит

1. Агент не понимает что некоторые окна — его «память», не просто инструменты
2. Система не защищает окна — любой `close` или `text` к любому окну проходит
3. Системный промпт говорит про meaningful titles, но не запрещает закрывать task

### Решение: Защищённые окна (Protected Windows)

**Реализация в `core/mod.rs`, `execute_blocks`**:

```rust
// Server struct добавляет:
protected_titles: Vec<String>,

// В execute_block_launch:
async fn execute_block_launch(&mut self, title: &str, mode: &BlockMode, content: &str) -> ExecutedBlock {
    // Проверка: защищённое ли окно
    if self.is_protected(title) {
        match mode {
            BlockMode::Close => {
                return ExecutedBlock {
                    title: title.to_string(),
                    window_id: None,
                    mode: *mode,
                    result: Some(format!(
                        "[PROTECTED] Cannot close window '{title}'. It is protected.
                         Use a different window title instead."
                    )),
                };
            }
            BlockMode::Text => {
                return ExecutedBlock {
                    title: title.to_string(),
                    window_id: None,
                    mode: *mode,
                    result: Some(format!(
                        "[PROTECTED] Cannot overwrite window '{title}'. 
                         It is protected and persists across ticks.
                         Create a new window with a different title."
                    )),
                };
            }
            BlockMode::Write => { /* write to file is fine, different mechanism */ }
        }
    }
    // ... остальная логика
}
```

**Начальный набор защищённых окон**: `["task"]` — или любое окно, заданное
в начальном промпте.

**Обновление системного промпта** (`src/prompt/system_prompt.txt`):
Добавить в правила:

```
## Protected Windows
Some windows are protected and cannot be closed or overwritten:
- `task` — your current task description, persists across ticks
- Any window marked as protected in the dashboard

If you try to close or overwrite a protected window, your command will be
rejected with an error. Use a different window title for your commands.
```

**В dashboard** помечать защищённые окна:
```
id 2 | task (PROTECTED) | pid 18273 | prompt: false
```

### Приоритет: HIGH — без этого агент регулярно теряет задачу

---

## Проблема 2: Агент не ведёт дневник действий и интентов

### Описание

Агент работает tick-by-tick без какой-либо памяти между тиками. Каждый тик
он получает содержимое окон и должен восстановить контекст с нуля. Это приводит к:

- Повторным действиям (агент забыл что уже сделал)
- Потере фокуса (агент забыл зачем начал что-то)
- Неспособности планировать многошаговые операции
- Неэффективному использованию контекстного окна

### Почему происходит

Текущая реализация:
- `last_tick: Option<(AgentResponse, String)>` — хранит только предыдущий ответ
- Нет `mind.md` (описан в архитектуре, но не реализован)
- Нет persistent storage между тиками
- Окна перезапускаются каждый тик (`rerun_watch_windows`) — что разрушает
  их как источник памяти

### Решение: Mind File + Action Journal

#### A. Mind File (`mind.md`)

Файл `sessions/default/mind.md`, читаемый каждый тик, включаемый в промпт
как часть Immutable Layer. Структура:

```markdown
# Mind

## Current Goal
Анализировать код TAI и написать план улучшений

## Completed Steps
- [x] Прочитал TASK.md
- [x] Изучил архитектуру
- [x] Прочитал исходный код
- [x] Выявил три критических проблемы

## Next Steps
- [ ] Написать plan_improving.md
- [ ] Закрыть все терминалы

## Notes
- AGENTS.md и ARCHITECTURE.md в корне устаревшие
- Рабочие файлы: src/_ и ACTUAL_STATE.md
```

**Реализация**: новый модуль `src/mind/mod.rs`

```rust
pub struct Mind {
    path: PathBuf,
    content: String,
}

impl Mind {
    pub fn load(path: &Path) -> Result<Self, MindError> {
        let content = if path.exists() {
            fs::read_to_string(path)?
        } else {
            String::new()
        };
        Ok(Self { path: path.to_path_buf(), content })
    }

    pub fn content(&self) -> &str { &self.content }
    
    pub fn update(&mut self, new_content: String) -> Result<(), MindError> {
        self.content = new_content;
        fs::write(&self.path, &self.content)?;
        Ok(())
    }
}
```

**Интеграция в `Prompt::build`**:
```rust
// В Prompt struct:
pub mind: Option<String>,

// В Prompt::build:
let mind = Mind::load(&mind_path).ok();
// ... включить в промпт
```

**Интеграция в tick**:
```rust
// После получения ответа агента — обновить mind:
if let Some(ref reasoning) = response.reasoning {
    // reasoning домена — кандидат для mind.md
    // Но пусть агент сам решает через специальный блок
}
```

#### B. Журнал действий (Action Journal)

Автоматический лог, который агент видит в каждом промпте:

```markdown
## Recent Actions (last 5 ticks)
1. [tick 3] Launched window `read1`: cat src/lib.rs → OK (exit 0)
2. [tick 3] Launched window `read2`: cat src/backend/mod.rs → OK (exit 0)
3. [tick 4] Read windows read1, read2, read3 → OK
4. [tick 4] Write to plan_improving.md → OK
5. [tick 5] Closed windows read1-read7, tree → OK
```

**Реализация**: В `Server` добавить:

```rust
pub struct ActionLog {
    entries: Vec<ActionEntry>,
    max_entries: usize,  // default: 20
}

pub struct ActionEntry {
    tick: u64,
    action: String,      // "Launched window 'read'"
    result: String,      // "OK (exit 0)" or "Error: ..."
    timestamp: Instant,
}
```

Каждый `execute_block` добавляет запись. В `Prompt::build` журнал
включается как Ephemeral Layer.

#### C. Пересмотр `rerun_watch_windows`

**Текущая проблема**: `rerun_watch_windows` перезапускает ВСЕ отслеживаемые
окна каждый тик, отправляя `clear && <original_command>`. Это означает:

- Окна с командой `cat TASK.md` перечитывают файл каждый тик — ОК
- Но окна с `tree --gitignore` тоже перезапускаются — расточительно
- Агент не может иметь окна с_long-running_ процессами (сервер, watch)
- Агент теряет возможность хранить持久状态 в окне

**Решение**: Разделить окна на категории:

| Категория | Поведение | Пример |
|-----------|-----------|--------|
| **Tracked** | Rerun каждый тик | `cat TASK.md`, `ls src/` |
| **Persistent** | Не перезапускается, живёт между тиками | `cargo watch`, сервер |
| **Protected** | Не закрывается агентом, не перезаписывается | `task`, `mind` |

Агент указывает категорию через mode_suffix или отдельный механизм:

```
```task:tracked
cat TASK.md     # перезапускается каждый тик
```

```server:persistent
cargo run       # запускается один раз, живёт
```
```

Для MVP достаточно двух режимов: tracked (по умолчанию) и persistent.

### Приоритет: CRITICAL — без памяти агент не может планировать

---

## Проблема 3: Агент воображает результаты невыполненных команд

### Описание

Агент иногда пишет команды в блоки кода, но в своём рассуждении (prose)
уже обсуждает результаты этих команд, как будто они выполнены. Или система
не выполнила блок, а агент в следующем тике действует так, будто всё прошло
успешно.

Это связано с двумя факторами:
1. LLM склонен «забегать вперёд» и описывать ожидаемые результаты как фактические
2. Система не чётко разделяет «план» и «результат» в промпте

### Примеры

**Случай A**: Агент пишет:
> «Проверю что билд чистый» и отправляет `cargo build 2>&1` в окно `build`.
> Затем продолжает: «Билд прошёл успешно, теперь запущу тесты...»
> Но билд ещё даже не выполнился!

**Случай B**: Агент воображает что команда выполнилась, потому что
он её «написал» в предыдущем ответе. На следующем тике он видит
свой предыдущий ответ с командой, но результат выполнения может
быть другим (или не отобразиться из-за бага).

### Решение: Чёткое разделение Plan vs Result в промпте

#### A. Реструктуризация промпта

Текущая структура (`src/agent/llm.rs`, `prompt_to_messages`):

```
1. System message (system prompt)
2. Window observations (focused windows)
3. Assistant message (previous response)  ← ПРОБЛЕМА: план и результат смешаны
4. User message (execution feedback)       ← может быть неясным
```

Предлагаемая структура:

```
1. System message (system prompt + mind + protected status)
2. User message: "## What you PLANNED last tick"
   - assistant response text (clearly labeled as PLAN, not FACT)
3. User message: "## What ACTUALLY happened"
   - execution feedback with clear pass/fail markers
4. User message: "## Current state (observations)"
   - dashboard + focused windows
```

Ключевое: **предыдущий ответ агента представлен как ПЛАН, а не как факт**.
И **результаты выполнения чётко отделены от наблюдений**.

#### B. Явная разметка результатов в feedback

Текущий `collect_feedback`:
```
[build] exit 0
cargo build output...
```

Предлагаемый формат:
```
## Execution Results (these ACTUALLY happened)
✅ [build] Command completed successfully (exit 0)
<output>

❌ [test] Command failed (exit 1)  
<output>

⚠️ [task] PROTECTED - command rejected
```

Для каждого блока — ясный статус:
- ✅ Выполнено успешно
- ❌ Ошибка выполнения
- ⚠️ Отклонено (защищённое окно, невалидный блок)

#### C. «No-speculation» правило в системном промпте

Добавить в system_prompt.txt:

```
## Critical Rule: No Speculation

NEVER describe the results of a command you haven't seen the output of.
When you send a command in a code block, you will see its results on the
NEXT tick. Until then, you do NOT know what happened.

In your prose (text outside blocks), you may state your INTENT:
  ✅ "I'm running cargo build to check if the code compiles."
  ❌ "Cargo build succeeded, now I'll run tests."  ← You don't know this yet!

Wait for the next tick's execution results before drawing conclusions.
```

#### D. Валидация ожиданий (future)

Более продвинутый вариант: агент может в своём ответе указать ожидания:

```markdown
```build:text
cargo build 2>&1
```
I expect: exit 0, no errors
```

Система сравнивает ожидания с реальными результатами и инжектит
предупреждение при расхождении. Но это Phase 7+ (prompt assembly).

### Приоритет: HIGH — LLM регулярно галлюцинирует результаты

---

## Порядок реализации

### Phase 1: Protected Windows (СРОЧНО)

1. Добавить `protected_titles: Vec<String>` в `Server`
2. В `execute_block_launch` проверять защиту перед выполнением
3. Обновить system prompt: правило о protected windows
4. В `build_dashboard` помечать защищённые окна
5. Начальный список: `["task"]`
6. Тест: агент пытается закрыть task → получает отказ

**Файлы**: `src/core/mod.rs`, `src/prompt/mod.rs`, `src/prompt/system_prompt.txt`

### Phase 2: Action Journal (СРОЧНО)

1. Добавить `ActionLog` структуру в `src/core/mod.rs`
2. Записывать каждое действие (launch, close, write) в журнал
3. Включать последние N записей в промпт
4. Очищать журнал при новом тике (или скользящее окно)

**Файлы**: `src/core/mod.rs`, `src/prompt/mod.rs`

### Phase 3: Prompt Restructuring (HIGH)

1. Переструктурировать `prompt_to_messages` в `src/agent/llm.rs`
2. Чётко разделить: previous plan → actual results → current observations
3. Добавить ✅/❌/⚠️ разметку в feedback
4. Добавить «No Speculation» правило в system prompt

**Файлы**: `src/agent/llm.rs`, `src/core/mod.rs`, `src/prompt/system_prompt.txt`

### Phase 4: Mind File (HIGH)

1. Создать `src/mind/mod.rs` с `Mind` struct
2. Читать `mind.md` в начале каждого тика, включать в промпт
3. Позволить агенту обновлять mind через специальный блок или автоматически
4. Хранить в `sessions/default/mind.md`

**Файлы**: `src/mind/mod.rs`, `src/lib.rs`, `src/prompt/mod.rs`, `src/core/mod.rs`

### Phase 5: Window Categories (MEDIUM)

1. Расширить `BlockMode` или добавить `WindowCategory` (tracked/persistent)
2. Persistent окна не перезапускаются каждый тик
3. Обновить `Watcher` для учёта категорий
4. Обновить system prompt

**Файлы**: `src/types.rs`, `src/backend/watch.rs`, `src/core/mod.rs`

---

## Дополнительные наблюдения о текущей реализации

### `rerun_watch_windows` — расточительность

Текущий `rerun_watch_windows` перезапускает ВСЕ отслеживаемые окна каждый
тик. Для окон типа `cat TASK.md` это нормально (файл может измениться),
но для окон типа `tree --gitignore` это лишняя работа. Кроме того,
это ПЕРЕЗАПИСЫВАЕТ вывод, так что агент никогда не видит «старый» вывод —
только свежий. Это одновременно и фича (всегда актуальные данные),
и проблема (невозможно отслеживать изменения).

### `wait_for_windows` — блокировка

`wait_for_windows` блокирует тик до завершения команд или таймаута (60с).
Это значит что если одна команда висит, весь тик висит. Нужно переходить
на async модель: watcher background task детектирует завершение окон
и триггерит новый тик через channel.

### `collect_feedback` — потеря контекста

`collect_feedback` собирает выводы окон, но теряет информацию о том,
какой блок относится к какому окну. Если агент отправил 3 блока,
а потом видит `[build] exit 0 | [test] exit 1 | [lint] exit 0`,
ему нужно соответствие между блоками и результатами.
Сейчас это работает через title, но нет гарантии уникальности.

### Отсутствие `bash -c` обёртки

PLAN.md отмечает что `last_cmd_exit_status` корректно работает только
если процесс — bash (или zsh). Текущий `launch_and_send` запускает `zsh`
без `bash -c`, что может приводить к потере exit code для некоторых команд.

### Window ID vs Title — коллизии

Текущий `get_id_for_title` ищет по title, но title не гарантированно
уникален. Если агент создаёт два окна с одинаковым title, `close`
закроет первое найденное. Нужно либо требовать уникальность title,
либо использовать числовые ID в протоколе.

---

## Резюме

Три критических проблемы агента и их решения:

| Проблема | Причина | Решение | Приоритет |
|----------|---------|--------|-----------|
| Закрывает/перезаписывает task | Нет защиты окон | Protected Windows | СРОЧНО |
| Не помнит что делает | Нет persistent памяти | Mind File + Action Journal | СРОЧНО |
| Воображает результаты | План и результат смешаны | Prompt Restructuring | HIGH |

Все решения вписываются в текущую архитектуру. Protected windows —
проверка в `execute_blocks`. Mind file — новый модуль + интеграция
в промпт. Prompt restructuring — изменения в `prompt_to_messages`
и системном промпте.
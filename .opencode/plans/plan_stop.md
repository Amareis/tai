# Plan: Agent-Initiated Cycle Stop

## Problem

TAI runs an infinite tick loop in `Server::run()` (src/core/mod.rs):

```rust
loop {
    if self.debug { let _ = lines.next_line().await; }
    self.tick().await?;
}
```

There is no mechanism for the agent (me) to signal "I'm done, stop the cycle." The agent can execute commands and write files, but cannot break out of the loop.

## Current Exit Points

- **Ctrl+C** (SIGINT) — kills the whole process, no cleanup
- **Debug mode** — manual Enter per tick, but still infinite
- **LLM API error** — `AgentError` propagates up, but this is accidental not intentional

## Proposed Solution: `BlockMode::Stop`

### 1. Add `Stop` variant to `BlockMode` (src/types.rs)

```rust
pub enum BlockMode {
    Text,   // send to stdin
    Close,  // close window
    Write,  // write file directly
    Stop,   // signal: stop the tick cycle
}
```

### 2. Parse `tai:stop` blocks (src/response/mod.rs)

The block syntax ````tai:stop\n``` or ````tai:stop\nreason text\n``` parses as:

```rust
ParsedSegment::Block {
    window: "tai".to_string(),
    mode: BlockMode::Stop,
    content: "reason text".to_string(),  // optional stop reason
}
```

`FromStr` for `BlockMode` gains `"stop" => Ok(Self::Stop)`.

### 3. Handle `Stop` in `execute_block` (src/core/mod.rs)

```rust
BlockMode::Stop => {
    // Signal graceful shutdown
    ExecutedBlock {
        title: "tai".into(),
        window_id: None,
        mode: BlockMode::Stop,
        result: Some(format!("[tai] stop requested: {content}")),
    }
}
```

### 4. Break the loop in `Server::run()`

After `execute_blocks`, check if any block was `Stop`:

```rust
loop {
    if self.debug { let _ = lines.next_line().await; }
    self.tick().await?;  // tick now returns Result<bool, CoreError>

    if self.should_stop {  // set by Stop block
        info!("stop requested, breaking cycle");
        break;
    }
}
```

### 5. Manifold approach: separate stop channel

Cleaner: add a `tokio::sync::watch` or `AtomicBool` stop flag:

```rust
pub struct Server {
    back: Box<dyn TerminalBackend>,
    agent: Box<dyn Agent>,
    watcher: Watcher,
    pub debug: bool,
    last_tick: Option<(AgentResponse, String)>,
    stop_requested: Arc<AtomicBool>,  // NEW
}
```

When `execute_block` encounters `Stop`, it sets `stop_requested` to `true`.
After `tick()`, `run()` checks the flag and breaks.

### 6. Update system prompt (src/prompt/mod.rs)

Add to SYSTEM_PROMPT:

```
```tai:stop\n``` — stop the tick cycle (graceful shutdown)
```tai:stop\nreason text\n``` — stop with a reason
```

## Simpler Alternative: File-based Stop Signal

If we don't want to modify `BlockMode`, we could use a convention:

1. Agent writes a `.tai_stop` file (using existing `Write` mode)
2. `Server::run()` checks for `.tai_stop` before each tick
3. If found, break the loop and delete the file

```rust
// In run() loop:
if std::path::Path::new(".tai_stop").exists() {
    info!("stop file found, breaking cycle");
    let _ = std::fs::remove_file(".tai_stop");
    break;
}
```

Agent usage:
```
```tai_stop:write<<EOF
task complete
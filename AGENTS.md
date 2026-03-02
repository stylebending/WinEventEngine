# AGENTS.md - Coding Guidelines for WinEventEngine

## Build Commands

```bash
# Build entire workspace
cargo build

# Build release
cargo build --release

# Build specific crate
cargo build -p engine_core
cargo build -p engine

# Check (faster than build)
cargo check
cargo check --all
```

## Test Commands

```bash
# Run all tests
cargo test

# Run tests for specific crate
cargo test -p engine_core
cargo test -p bus

# Run a single test by name
cargo test it_works
cargo test it_works -p engine_core

# Run tests with output
cargo test -- --nocapture

# Run ignored tests
cargo test -- --ignored
```

## Lint Commands

```bash
# Run Clippy (linting)
cargo clippy
cargo clippy --all-targets --all-features

# Fix auto-fixable issues
cargo clippy --fix

# Check formatting
cargo fmt -- --check

# Format code
cargo fmt
```

## Workspace Structure

- `engine/` - Main application binary
- `engine_core/` - Core types (Event, EventKind, Plugin traits)
- `bus/` - Event bus/channel implementation
- `rules/` - Rule matching logic
- `actions/` - Action execution
- `metrics/` - Metrics collection and HTTP endpoint
- `core/` - Additional core utilities

## Code Style Guidelines

### Imports
- Group imports: std lib first, then external crates, then internal modules
- Use `use crate::` for internal imports
- Example:
  ```rust
  use std::time::Instant;
  use tokio::sync::mpsc;
  use crate::event::Event;
  ```

### Naming Conventions
- `PascalCase` for types, traits, enums, structs
- `snake_case` for functions, variables, modules
- `SCREAMING_SNAKE_CASE` for constants
- `PascalCase` for enum variants

### Types & Traits
- Derive common traits: `Debug`, `Clone`, `PartialEq`, `Eq` where applicable
- Use `pub` explicitly for exports
- Prefer struct fields to be `pub` when part of public API
- Use `#[async_trait]` for async trait methods

### Error Handling
- Use `Result` for recoverable errors
- Use `expect()` or `unwrap()` only in tests or when truly infallible
- Propagate errors with `?` operator
- Use `let _ =` when intentionally ignoring results

### Async Patterns
- Use `tokio` runtime (`#[tokio::main]`)
- Prefer `mpsc` channels for communication
- Use `async_trait::async_trait` for trait async methods

### Formatting
- 4 spaces for indentation
- Max line length: 100 characters (default rustfmt)
- Trailing commas in multi-line structs/enums

### Testing
- Place tests in `#[cfg(test)]` module at bottom of file
- Use `use super::*;` in test modules
- Name tests descriptively (e.g., `it_works`, `test_event_matching`)

## Dependencies

- `tokio` - Async runtime (use `features = ["full"]` for binaries, minimal for libs)
- `uuid` - UUID generation (`features = ["v4"]`)
- `async-trait` - Async trait support

## Edition

This project uses Rust 2024 edition.

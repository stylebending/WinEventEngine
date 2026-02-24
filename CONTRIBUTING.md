# Contributing to Windows Event Automation Engine

Thank you for your interest in contributing! This document will help you get started.

## Getting Started

1. **Fork the repository** on GitHub
2. **Clone your fork**:
   ```bash
   git clone https://github.com/stylebending/wineventengine.git
   cd wineventengine
   ```
3. **Install prerequisites**:
   - [Rust](https://rustup.rs/) (latest stable)
   - [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/) (Windows only)

## Development Setup

```bash
# Build the project
cargo build --release

# Run tests
cargo test

# Run with example config
cargo run --release -- -c config.toml.example
```

## Code Style

- Follow standard Rust formatting (`cargo fmt`)
- Run clippy before committing (`cargo clippy`)
- Keep functions focused and documented
- Use meaningful variable names

## Testing

```bash
# Run all tests
cargo test

# Run specific crate tests
cargo test -p engine
cargo test -p actions

# Run with output
cargo test -- --nocapture
```

## Submitting Changes

1. **Create a branch** for your feature/fix:
   ```bash
   git checkout -b feature/my-feature
   ```

2. **Make your changes** and test them

3. **Commit with clear messages**:
   ```bash
   git commit -m "Add feature X to do Y"
   ```

4. **Push to your fork**:
   ```bash
   git push origin feature/my-feature
   ```

5. **Open a Pull Request** on GitHub with:
   - Clear description of changes
   - Why the change is needed
   - Testing performed

## Areas for Contribution

- Bug fixes
- New event source plugins
- New action types
- Documentation improvements
- Lua API enhancements
- Performance optimizations

## Questions?

- Open an issue for bugs or feature requests
- Check existing issues and PRs first
- Join discussions in open issues

## License

By contributing, you agree that your contributions will be licensed under the MIT License.

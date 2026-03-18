<p align="center"><img width="150px" heigth="150px" src="assets/logo.png"></p>

<p align="center"><img src="https://readme-typing-svg.herokuapp.com?font=Fira+Code&pause=1000&center=true&width=435&lines=WinEventEngine;Simple+Configuration;Powerful+Results" alt="WinEventEngine" /></p>

<p align="center"><a href="https://github.com/stylebending/WinEventEngine/releases"><img src="https://img.shields.io/github/downloads/stylebending/WinEventEngine/total?color=darkgreen&logo=github&label=Github%20Downloads&style=for-the-badge&labelColor=darkgreen"></a></p>

<br>

<h3 align="center">Quick Navigation</h3>

<p align="center"><a href="#quick-start"><img src="https://img.shields.io/badge/🚀-Quick%20Start-darkblue?style=for-the-badge&labelColor=darkblue"></a> <a href="https://github.com/stylebending/WinEventEngine/wiki"><img src="https://img.shields.io/badge/📖-Documentation%20Wiki-darkblue?style=for-the-badge&labelColor=darkblue"></a></p>

<p align="center"><a href="https://github.com/stylebending/WinEventEngine/blob/main/LICENSE"><img src="https://img.shields.io/badge/📄-MIT%20License-darkblue?style=for-the-badge&labelColor=darkblue"></a> <a href="https://github.com/stylebending/WinEventEngine/blob/main/CONTRIBUTING.md"><img src="https://img.shields.io/badge/🪽-Contributing-darkblue?style=for-the-badge&labelColor=darkblue"></a></p>

<br>

## 📦 What is WinEventEngine?

WinEventEngine is an event-driven automation framework for Windows with a GUI:
- Play/pause media when focusing specific windows
- Auto commit changes to your config/dot files/folders
- Auto build/test an application under configurable conditions
- Get Webhook (Discord/Slack/Telegram/etc) notifications for configurable events
- Send API requests for configurable conditions/events
- Write easy-to-learn Lua scripts to customize everything
- Manage everything through an intuitive native GUI
- Much more! Simple configuration, powerful results

## Key Features

- **Native GUI**: Built with Iced for a modern, responsive interface
- **Real-time Dashboard**: Monitor events, matches, and actions live
- **Event Monitoring**: File system, windows, processes, and registry
- **Rule Engine**: Pattern-based matching with Lua scripting
- **Windows Service**: Run as background service
- **Plugin System**: Write custom actions in Lua
- **Multiple Themes**: Dark, Light, and System theme support
- **Security**: Optional HTTP request controls, admin-only service operations

## Quick Start

### Download & Run

1. Download the latest release from [GitHub Releases](https://github.com/stylebending/WinEventEngine/releases)
2. Run `WinEventEngine.exe` to launch the GUI
3. Or run `WinEventEngine.exe --cli` for command-line mode

### GUI Mode (Default)

The GUI provides an intuitive interface for:
- **Dashboard**: Real-time monitoring of events and metrics
- **Automations**: Create, edit, and manage automation rules
- **Event Sources**: Configure file watchers, window watchers, and more
- **Event Tester**: Test rules against sample events
- **Settings**: Manage themes, service installation, HTTP security, and configuration

**Learn more**: [GUI Guide](https://github.com/stylebending/WinEventEngine/wiki/GUI-Guide)

### CLI Mode

Use command-line flags for automation and scripting:

```bash
# Show help
WinEventEngine.exe --help

# Run with specific config
WinEventEngine.exe --cli --config myconfig.toml

# Install as Windows Service
WinEventEngine.exe --install

# Check status
WinEventEngine.exe --status
```

## 📖 Documentation

- 💬 [Discord](https://discord.gg/tv65exPKgP) - Community support
- 📖 [Wiki](https://github.com/stylebending/WinEventEngine/wiki) - Full documentation
- 💡 [Discussions](https://github.com/stylebending/WinEventEngine/discussions) - Feature requests
- 🐛 [Issues](https://github.com/stylebending/WinEventEngine/issues) - Bug reports

## 🛠️ Building from Source

```bash
# Clone the repository
git clone https://github.com/stylebending/WinEventEngine.git
cd WinEventEngine

# Build the GUI
cargo build --release -p WinEventEngine

# Build CLI only
cargo build --release -p WinEventEngine

# Run tests
cargo test --workspace
```

## 🤝 Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

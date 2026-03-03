# Tako Desktop

Tako Desktop is a native macOS application that bundles the [Tako](https://tako.shiroha.tech) Hub server, CLI, and Web UI into a single `.app`. It provides a seamless desktop experience for managing AI coding tools like Claude Code and Codex.

Built with [Tauri](https://tauri.app/) (Rust + WebView).

## Install

Download the latest `.dmg` from [**Releases**](https://github.com/Barrierml/tako-desktop/releases):

| Architecture | File |
|---|---|
| Apple Silicon (M1/M2/M3/M4) | `Tako_*_aarch64.dmg` |
| Intel | `Tako_*_x64.dmg` |

Open the `.dmg`, drag **Tako** to Applications, and launch.

## Build from Source

### Prerequisites

- [Bun](https://bun.sh/) >= 1.0
- [Rust](https://rustup.rs/) (stable)
- [Tauri CLI](https://tauri.app/start/): `cargo install tauri-cli`

### Steps

```bash
git clone https://github.com/Barrierml/tako-desktop.git
cd tako-desktop

bun install
bun run prepare   # Downloads pre-built bundles from npm
cargo tauri build  # Produces .app and .dmg
```

The `.dmg` will be at `src-tauri/target/release/bundle/dmg/`.

## Architecture

```
tako-desktop/
├── scripts/
│   └── prepare-resources.ts   # Downloads/builds Hub + CLI + Web bundles
├── src-tauri/
│   ├── src/
│   │   ├── main.rs            # Tauri app entry point
│   │   ├── lib.rs             # Setup: Bun install → Hub loop
│   │   ├── bun_manager.rs     # Bun runtime management
│   │   ├── bundle_resolver.rs # Bundle path resolution (built-in / hot-update)
│   │   ├── hub_manager.rs     # Hub process lifecycle
│   │   └── region.rs          # Region detection (mirrors for CN users)
│   ├── resources/
│   │   └── loading/           # Splash screen shown during startup
│   └── tauri.conf.json        # Tauri configuration
└── package.json
```

### How It Works

1. **Startup**: Tauri shows a loading screen while the Rust backend initializes
2. **Bun Runtime**: Ensures a dedicated Bun runtime is installed at `~/.tako/bun/`
3. **Hub Server**: Launches the bundled Hub server (API gateway + Web UI) on `localhost:3006`
4. **WebView**: Navigates the native window to the Hub web interface
5. **Hot Restart**: Hub can exit with code 42 to trigger a restart with updated bundles

### Bundle Resolution

Bundles are resolved with external-first priority:
- **External** (`~/.tako/desktop-bundles/`): Hot-updated bundles downloaded at runtime
- **Built-in** (app resources): Bundles shipped inside the `.app`

This allows the app to self-update its Hub/CLI/Web components without a full app update.

## License

[AGPL-3.0](LICENSE)

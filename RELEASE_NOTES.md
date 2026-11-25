Azeroth Survivor Reborn Trainer v1.1.5

- Ship as a macOS Apple Silicon app only. The UI is rewritten in SwiftUI with a System Settings-style sidebar.
- Keep character discovery and stat apply in a Rust static library bridged to Swift.
- Publish a single stable zip: `Azeroth.Survivor.Reborn.Trainer.macos-arm64.zip`. Updates open the GitHub release page.

Azeroth Survivor Reborn Trainer v1.1.4

- Add an Apple Silicon build with combat stat discovery, read-only refresh, and individual Apply.
- Refresh the macOS interface and app icon. English and Simplified Chinese, light and dark appearance.
- Publish the macOS ZIP as a separate prerelease. The Windows stable release included the standalone EXE with self-update support.

Azeroth Survivor Reborn Trainer v1.1.3

- Memory Read mode and the `--mode` and `--find` command-line arguments are removed. Find always locates the Stats panel with OCR, then reads and sets values in memory.
- Once the Stats panel is found, memory discovery makes up to six attempts.
- Open the Character panel (press C) while Find is running. The trainer asks for it in its window.

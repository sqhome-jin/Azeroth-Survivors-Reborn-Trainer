# Azeroth Survivor Reborn Trainer for MacOS

macOS Apple Silicon trainer for solo runs in [Azeroth Survivors: Reborn](https://azreborn.com/).
Find your character, read current combat stats, and apply individual targets.

[中文说明](README.md)

| Platform | Download |
| --- | --- |
| macOS Apple Silicon | [Latest release](https://github.com/sqhome-jin/Azeroth-Survivors-Reborn-Trainer/releases/latest) |

## Use the trainer

1. Download `Azeroth.Survivor.Reborn.Trainer.macos-arm64.zip`, extract it, and move **Azeroth Survivor Reborn Trainer.app** to Applications.
2. Start Reborn through its official launcher, then open the trainer.
3. Choose the stats to enable and set their targets.
4. Start a run and click **Find**. The trainer discovers the character in memory and applies each enabled target once after verifying the selected character and stat mapping.
5. Use **Apply** to set the selected stat again, or **Read** to refresh values without writing.

## Game updates and stat discovery

**Find** looks for your character in memory and finds the stat block by analysing
the game executable. The combat stats (Damage, Crit, Crit Damage, Speed, Armor,
Regen, Pickup, Gold Bonus, Area, Targets, Luck) are placed inside that block, so a
new game build does not need hand-entered offsets for them. Health, Run Gold,
Rerolls, Cooldown reduction, and Echo need a verified mapping for each build and
may be unavailable.

Current health follows verified maximum health. Echo requires the active Echo effect.

## Troubleshooting

Open **Diagnostics** to copy the game build, process, session state, per-stat
readings and targets, and recent log lines. The background log is at:

`~/Library/Application Support/Azeroth Survivor Reborn Trainer/reborn-rust.log`

The active log is capped at 10 KB and rotates to `reborn-rust.previous.log`. If
settings are malformed, the trainer preserves the original and loads defaults.

```bash
tail -n 40 -f ~/Library/Application\ Support/Azeroth\ Survivor\ Reborn\ Trainer/reborn-rust.log
```

The app is ad-hoc signed with the debugger entitlement so it can attach to the
game. macOS may require approval under **System Settings → Privacy & Security**
after the first launch, or authorization when it attaches to the game. If
Gatekeeper blocks the unzipped app:

```bash
xattr -d com.apple.quarantine "/Applications/Azeroth Survivor Reborn Trainer.app"
```

The trainer looks for the game at
`~/Library/Application Support/Azeroth Survivors/AzerothSurvivorsGame` by default.
Use **Locate…** if your install path differs.

Use the sidebar preferences for English or Simplified Chinese and System, Light,
or Dark appearance. Selections are remembered.

## Updates

The trainer checks GitHub when it starts. When an update is available, open the
GitHub release page from the banner and download the new zip.

## Repository

- `src/` contains the Rust backend: process access, hunt/apply, settings, logging, and the C ABI.
- `macos/Trainer/` contains the SwiftUI host. Build with `./macos/build-app.sh`.
- `assets/` contains the app icon and logo.
- `.github/workflows/release.yml` builds, tests, signs, and publishes the macOS zip on `main`.

## Related links

- Original Windows trainer: [nelbin4 / Azeroth-Survivors-Reborn-Trainer Releases](https://github.com/nelbin4/Azeroth-Survivors-Reborn-Trainer/releases)
- License: [LICENSE](LICENSE)

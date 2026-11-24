# Azeroth Survivor Reborn Trainer

Standalone Windows x64 trainer for [Azeroth Survivors: Reborn](https://azreborn.com/),
written in Rust. [Download the latest standalone EXE](https://github.com/nelbin4/Azeroth-Survivors-Reborn-Trainer/releases/latest)

## Use the trainer

1. Run **Azeroth.Survivor.Reborn.Trainer.exe**, then start Reborn.
2. Choose the stats in the **Enabled** column and set their targets in the side panel.
3. Start a run and click **Find**. While the trainer is finding stats, open the in-game Character panel with **C** and leave it open until the trainer finds the values. The trainer then applies each enabled target once after verifying the selected character and stat mapping.
4. Use **Apply** to set the selected stat again. Use **Find** to apply all enabled targets again.

### Game updates and stat discovery

**Find** looks for your character in memory and finds the stat block by analysing
the game executable. The combat stats (Damage, Crit, Crit Damage, Speed, Armor,
Regen, Pickup, Gold Bonus, Area, Targets, Luck) are placed inside that block, so a
new game build does not need hand-entered offsets for them. Build `c639287405d0`
has resource and combat mappings. Cooldown reduction and Echo are unavailable on
this build. Health, Run Gold and Rerolls need a verified mapping for each build.
Enable the targets you want before clicking **Find**, or select one and click
**Apply**. Targets outside the verified mapping are skipped.

Find keeps scanning the game window until the Stats panel appears, pausing two
seconds between scans. Open the in-game Character panel with **C** when the
trainer asks. OCR only locates the Stats panel; memory then finds the character
and sets the values. Once the panel is found, memory discovery makes up to six
attempts.

**Find** discovers the game process, character, and verified stat mapping.
Apply writes checked targets once and rechecks the selected character and
instruction mapping before each write. If Windows reports a partial read
(error 299), the scanner retries page by page. Click **Find** again if the
selected character changes or a scan is interrupted.

Current health follows verified maximum health. Echo requires the active Echo effect.

### Troubleshooting

Open **Advanced → Detection details** and click **Copy details** to copy the game
build, process, session state, last Apply error, per-stat readings and targets,
and up to 40 recent log lines. The log tail refreshes while Advanced is open.
Open the in-game **Character** panel and click **Read Stats panel** in Advanced
for a Windows OCR snapshot of the visible readings. The OCR snapshot is included
in **Copy details**. Apply uses verified memory mappings only.

The background log includes search, process detection, update, and stat events at
`%LOCALAPPDATA%\Azeroth Survivor Reborn Trainer\reborn-rust.log`.
The active log is capped at 10 KB (10,000 bytes) and rotates to `reborn-rust.previous.log`. If settings are malformed,
the trainer preserves the original and loads defaults so changes can still be saved.

```powershell
Get-Content -LiteralPath "$env:LOCALAPPDATA\Azeroth Survivor Reborn Trainer\reborn-rust.log" -Tail 40 -Wait
```

## Updates

The trainer checks GitHub when it starts. **Check updates** shows **Using Latest**
when the installed version is current. When **Update Available** appears, click
**Download & Install** to verify the update, replace the EXE, and reopen the trainer.
The installation folder needs to be writable. The updater keeps the previous EXE
as a backup. Starting with v1.0.1, the updater waits for the replacement to
confirm that its UI initialized. If startup fails, it restores and reopens the
previous EXE.

## macOS on Apple Silicon

Download the **Reborn-macOS-arm64** artifact from a successful GitHub Actions
run and extract the trainer ZIP inside it. Move **Azeroth Survivor Reborn
Trainer.app** to Applications. The app is ad-hoc signed with the debugger
entitlement so it can attach to the game. macOS may require you to approve the
app under **System Settings → Privacy & Security** after the first launch
attempt, or to authorize it when it attaches to the game.

Install and start the game through its official launcher. The trainer looks for
the game at `~/Library/Application Support/Azeroth Survivors/AzerothSurvivorsGame`
by default. Start a run and click **Find**. macOS discovers the character
directly from game memory. On game version 0.7.4, the verified layout covers
the combat stats listed above. Health, Run Gold, Rerolls, and screen OCR are
unavailable in the macOS build.

Download new macOS builds manually from Actions. The log is at
`~/Library/Application Support/Azeroth Survivor Reborn Trainer/reborn-rust.log`.

## Repository

- `src/` contains the Rust trainer, stat verification, Windows integration, and updater.
- `assets/` contains the app icon and logo.
- `.github/workflows/release.yml` builds and tests both platforms, uploads the macOS app as an Actions artifact, and publishes the standalone Windows EXE for newer versions on `main`.

### Local folders

- `releases/<version>/` contains downloaded standalone EXEs. Older Python release bundles and ZIP archives are grouped under `releases/legacy/`.
- `backups/workspace-cleanup/` contains archived diagnostics, test builds, generated caches, and the older local Reborn source.

The Python trainer remains available in the [v0.9.19 release and source tag](https://github.com/nelbin4/Azeroth-Survivors-Reborn-Trainer/releases/tag/v0.9.19).
See [LICENSE](LICENSE).

Azeroth Survivor Reborn Trainer v1.1.4

- Add an Apple Silicon build with combat stat discovery, read-only refresh, and individual Apply.
- Refresh the macOS interface and app icon. The native title bar includes English and Simplified Chinese, light and dark appearance, and Liquid Glass on macOS 26 and later.
- Publish the macOS ZIP as a separate prerelease. The Windows stable release includes the standalone EXE with self-update support.

Azeroth Survivor Reborn Trainer v1.1.3

- Memory Read mode and the `--mode` and `--find` command-line arguments are removed. Find always locates the Stats panel with OCR, then reads and sets values in memory.
- Once the Stats panel is found, memory discovery makes up to six attempts.
- Open the Character panel (press C) while Find is running. The trainer asks for it in its window.

Azeroth Survivor Reborn Trainer v1.1.2

- OCR is the default Detection Mode. Memory Read is still available in Advanced.
- OCR scans are faster: the no-op resize is skipped, and each scan logs its capture and recognize time (`ocr.scan` in the log).
- OCR waits 2 seconds between scans while the Stats panel is not on screen.
- The Character panel prompt pulses between gold and bright yellow so it is easy to notice.

Azeroth Survivor Reborn Trainer v1.1.1

- The Stats panel is recognised only when OFFENSE and DEFENSE sections sit under its heading, so other "Stats" text on screen is ignored.
- The Character panel prompt appears only while OCR is waiting for the Stats panel.
- README updated for Memory Read fallback to OCR and the `--mode` and `--find` options.

Azeroth Survivor Reborn Trainer v1.1.0

- With `--mode 2` (OCR), Find keeps scanning the screen until the Stats panel is found, pausing about one second between scans. Stop ends the scans. Memory discovery runs once after the panel is found.
- With the default Memory Read mode, if memory discovery fails its six attempts, Find switches to OCR, waits for the Stats panel, then tries memory discovery once more.

Azeroth Survivor Reborn Trainer v1.0.9

- Command-line options: `--mode 1` runs Memory Read and `--mode 2` runs OCR for that run only. `--find` starts Find once the game is connected. Neither option is saved.
- OCR scans the whole game window and finds the Stats panel from its heading.
- A large prompt asks you to open the Character panel (press C) while Find is running.

Azeroth Survivor Reborn Trainer v1.0.8

- Memory Read adapts to new builds: when the stat block is found by code analysis, the combat stats (Damage, Crit, Crit Damage, Speed, Armor, Regen, Pickup, Gold Bonus, Area, Targets, Luck) are located from that block. No hand-entered offsets are needed for these stats on a new build.
- Health, Run Gold, Rerolls and Echo still need verified mappings and are unavailable on builds that have not been verified.
- OCR is not used to find or write stats.

Azeroth Survivor Reborn Trainer v1.0.7

- Add a saved Detection Mode selector in Advanced: Memory Read (default) or OCR.
- Retry memory discovery up to five times after the initial attempt, then capture the visible Stats panel with OCR as a diagnostic fallback.
- OCR-first mode captures visible values before memory discovery. OCR readings do not verify writable memory fields; stat writes still require a verified mapping.
- Automatically apply a stat when its Enabled checkbox is checked and its live mapping is already verified. Enabled targets also continue to apply after Find completes.

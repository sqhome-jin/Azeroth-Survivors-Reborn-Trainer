# Azeroth Survivor Reborn Trainer for MacOS

适用于 Apple Silicon 的 macOS 修改器，用于 [Azeroth Survivors: Reborn](https://azreborn.com/) 单人局。
可查找角色、读取当前战斗属性，并分别应用目标值。

[English README](README.en.md)

| 平台 | 下载 |
| --- | --- |
| macOS Apple Silicon | [最新发布](https://github.com/sqhome-jin/Azeroth-Survivors-Reborn-Trainer/releases/latest) |

## 使用方法

1. 下载 `Azeroth.Survivor.Reborn.Trainer.macos-arm64.zip`，解压后将 **Azeroth Survivor Reborn Trainer.app** 移到「应用程序」。
2. 通过官方启动器启动游戏，再打开修改器。
3. 勾选要修改的属性，并设置目标值。
4. 进入一局后点击 **查找**。修改器会在内存中定位角色；在验证角色与属性映射后，对已勾选目标各写入一次。
5. 使用 **应用** 再次写入当前选中属性；使用 **读取** 仅刷新当前值，不写入。

## 游戏更新与属性发现

**查找** 会在内存中寻找角色，并通过分析游戏可执行文件定位属性块。战斗属性（伤害加成、暴击率、暴击伤害、移动速度、伤害减免、生命恢复、拾取范围、金币获取、作用范围、额外目标、幸运）位于该块内，因此新版本游戏通常无需手填这些偏移。当前生命值、本局金币、重掷次数、冷却缩减、回响概率需要按版本验证映射，部分版本可能不可用。

当前生命值跟随已验证的最大生命值。回响概率需要局内已有生效的回响效果。

## 故障排查

打开 **诊断** 可复制游戏构建、进程、会话状态、各属性读数与目标值，以及最近日志。后台日志位于：

`~/Library/Application Support/Azeroth Survivor Reborn Trainer/reborn-rust.log`

活动日志上限约 10 KB，轮转到 `reborn-rust.previous.log`。若设置文件损坏，修改器会保留原文件并加载默认值。

```bash
tail -n 40 -f ~/Library/Application\ Support/Azeroth\ Survivor\ Reborn\ Trainer/reborn-rust.log
```

应用以 ad-hoc 方式签名，并带有 debugger entitlement，才能附加到游戏进程。首次启动后，可能需要在 **系统设置 → 隐私与安全性** 中批准，或在附加游戏时授权。若 Gatekeeper 拦截解压后的应用：

```bash
xattr -d com.apple.quarantine "/Applications/Azeroth Survivor Reborn Trainer.app"
```

默认游戏路径为
`~/Library/Application Support/Azeroth Survivors/AzerothSurvivorsGame`。
路径不同时，请使用 **定位…**。

侧栏可切换英文 / 简体中文，以及跟随系统、浅色、深色外观；选择会自动保存。

## 更新

启动时会检查 GitHub。若有新版本，可通过横幅打开发布页并下载新的 zip。

## 仓库说明

- `src/`：Rust 后端（进程访问、查找/写入、设置、日志、C ABI）
- `macos/Trainer/`：SwiftUI 界面，可用 `./macos/build-app.sh` 打包
- `assets/`：图标与 logo
- `.github/workflows/release.yml`：在 `main` 上构建、测试、签名并发布 macOS zip

## 相关链接

- Windows 原版修改器：[nelbin4 / Azeroth-Survivors-Reborn-Trainer Releases](https://github.com/nelbin4/Azeroth-Survivors-Reborn-Trainer/releases)
- 许可证：[LICENSE](LICENSE)

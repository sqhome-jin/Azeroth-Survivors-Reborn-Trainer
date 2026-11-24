#[derive(Clone, Copy, Default)]
pub struct Language {
    pub chinese: bool,
}

impl Language {
    pub fn text<'a>(self, english: &'a str, chinese: &'a str) -> &'a str {
        if self.chinese { chinese } else { english }
    }

    pub fn status(self, english: &str) -> &str {
        let chinese = match english {
            "Start Reborn to find a character." => "请启动游戏以查找角色。",
            "Start a run, then click Find." => "请开始一局，再点按「查找」。",
            "Finding character and stats…" => "正在查找角色和属性…",
            "Character and stats found." => "已找到角色和属性。",
            "Search stopped." => "查找已停止。",
            "Apply cancelled." => "已取消应用。",
            "Applying stats…" => "正在应用属性…",
            "No character or stats were found." => "未找到角色或属性。",
            "Checking…" => "正在检查…",
            "Update Available" => "有可用更新",
            "Latest" => "已是最新版",
            "Update check failed" => "检查更新失败",
            "Downloading update…" => "正在下载更新…",
            "Update failed" => "更新失败",
            "Restarting…" => "正在重新启动…",
            "This stat is unavailable on the detected build." => "当前游戏版本无法修改此属性。",
            "Current health follows verified maximum health." => {
                "当前生命值跟随已验证的最大生命值。"
            }
            "Find applies checked targets once." => "查找会应用一次已勾选的目标值。",
            _ => english,
        };
        self.text(english, chinese)
    }
}

use serde_json::Value;

pub struct Stat {
    pub key: &'static str,
    pub label: &'static str,
    pub chinese_label: &'static str,
    pub min: u32,
    pub max: u32,
    pub percent: bool,
}

macro_rules! stat {
    ($key:literal, $label:literal, $chinese_label:literal, $min:literal, $max:literal, $percent:literal) => {
        Stat {
            key: $key,
            label: $label,
            chinese_label: $chinese_label,
            min: $min,
            max: $max,
            percent: $percent,
        }
    };
}

pub const STATS: [Stat; 17] = [
    stat!("damage", "Damage bonus", "伤害加成", 0, 9999, true),
    stat!("crit", "Critical strike", "暴击率", 8, 100, true),
    stat!(
        "crit_damage",
        "Critical damage",
        "暴击伤害",
        50,
        10049,
        true
    ),
    stat!("cooldown", "Cooldown reduction", "冷却缩减", 0, 80, true),
    stat!("area", "Area of effect", "作用范围", 0, 1000, true),
    stat!("targets", "Extra targets", "额外目标", 0, 32, false),
    stat!("echo", "Echo chance", "回响概率", 0, 100, true),
    stat!("health", "Current health", "当前生命值", 0, 1000000, false),
    stat!(
        "max_health",
        "Maximum health",
        "最大生命值",
        1,
        1000000,
        false
    ),
    stat!("armor", "Damage reduction", "伤害减免", 0, 60, true),
    stat!(
        "regen",
        "Regeneration bonus",
        "生命恢复加成",
        0,
        9999,
        false
    ),
    stat!(
        "speed",
        "Movement speed bonus",
        "移动速度加成",
        0,
        200,
        true
    ),
    stat!(
        "pickup",
        "Pickup range bonus",
        "拾取范围加成",
        0,
        9999,
        true
    ),
    stat!(
        "gold_bonus",
        "Gold find bonus",
        "金币获取加成",
        0,
        9999,
        true
    ),
    stat!("luck", "Luck bonus", "幸运加成", 0, 9999, true),
    stat!("run_gold", "Run gold", "本局金币", 0, 99999999, false),
    stat!("rerolls", "Rerolls", "重掷次数", 0, 9999, false),
];

impl Stat {
    pub fn checked(&self, prefs: Option<&Value>) -> bool {
        prefs
            .and_then(|data| data.get("enabled"))
            .and_then(Value::as_array)
            .map_or_else(
                || matches!(self.key, "damage" | "cooldown" | "area" | "luck"),
                |keys| keys.iter().any(|key| key.as_str() == Some(self.key)),
            )
    }

    pub fn saved_target(&self, prefs: Option<&Value>) -> u32 {
        prefs
            .and_then(|data| data.get("targets"))
            .and_then(|targets| targets.get(self.key))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| self.valid_target(*value))
            .unwrap_or(self.max)
    }

    pub fn valid_target(&self, target: u32) -> bool {
        (self.min..=self.max).contains(&target)
    }

    pub fn target(&self, value: u32) -> String {
        if self.key == "health" {
            return "Full".into();
        }
        let prefix = if self.key == "max_health" { "≥ " } else { "" };
        format!("{prefix}{}", self.display(u64::from(value)))
    }

    pub fn display(&self, value: u64) -> String {
        let unit = if self.percent { "%" } else { "" };
        format!("{}{unit}", grouped(value))
    }
}

pub fn grouped(number: u64) -> String {
    let number = number.to_string();
    let mut value = String::with_capacity(number.len() + number.len() / 3);
    for (index, digit) in number.chars().enumerate() {
        if index != 0 && (number.len() - index).is_multiple_of(3) {
            value.push(',');
        }
        value.push(digit);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uses_saved_targets_even_for_checked_stats() {
        let prefs =
            json!({"enabled": ["damage"], "targets": {"damage": 700, "crit": 42, "rerolls": -1}});
        let stat = |key| STATS.iter().find(|stat| stat.key == key).unwrap();
        assert_eq!(
            stat("damage").target(stat("damage").saved_target(Some(&prefs))),
            "700%"
        );
        assert_eq!(
            stat("crit").target(stat("crit").saved_target(Some(&prefs))),
            "42%"
        );
        assert_eq!(
            stat("rerolls").target(stat("rerolls").saved_target(Some(&prefs))),
            "9,999"
        );
        assert_eq!(stat("health").target(0), "Full");
        assert_eq!(
            stat("max_health").target(stat("max_health").saved_target(None)),
            "≥ 1,000,000"
        );
        assert!(stat("cooldown").checked(None));
        assert!(!stat("cooldown").checked(Some(&prefs)));
    }
}

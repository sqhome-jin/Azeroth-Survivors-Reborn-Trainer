//! Reads the in-game Stats panel from a screenshot of the game window.
//! `read` locates the panel with the platform OCR engine; `panel_text` turns
//! the recognized rows into the report shown in diagnostics.
#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::read;

/// Whether this platform can locate the Stats panel on screen. Find waits for
/// the panel only when OCR is supported.
pub const SUPPORTED: bool = cfg!(windows);

#[cfg(not(windows))]
pub fn read(_pid: u32) -> std::io::Result<String> {
    Err(std::io::Error::other(
        "Reading the Stats panel from the screen is not available on this platform yet.",
    ))
}

// Shared by every OCR backend; only the Windows one exists so far.
#[cfg_attr(not(windows), allow(dead_code))]
fn panel_text(rows: &str) -> Option<String> {
    let mut lines = rows
        .lines()
        .map(str::trim)
        .skip_while(|line| !line.eq_ignore_ascii_case("Stats"));
    lines.next()?;
    let mut section = "";
    let mut readings = Vec::new();
    for line in lines {
        if ["RUN", "OFFENSE", "DEFENSE", "UTILITY"]
            .iter()
            .any(|heading| heading.eq_ignore_ascii_case(line))
        {
            section = line;
            readings.push(line.to_owned());
            continue;
        }
        if section.is_empty() {
            continue;
        }
        let labels: &[&str] = if section.eq_ignore_ascii_case("RUN") {
            &["Time", "Experience", "Kills", "Gold", "Rerolls"]
        } else if section.eq_ignore_ascii_case("OFFENSE") {
            &[
                "Damage",
                "Critical strike",
                "Critical damage",
                "Cooldown reduction",
                "Area of effect",
                "Extra targets",
                "Echo",
            ]
        } else if section.eq_ignore_ascii_case("DEFENSE") {
            &["Health per second", "Health", "Damage reduction"]
        } else {
            &["Movement speed", "Pickup range", "Gold find", "Luck"]
        };
        if labels.iter().any(|label| {
            line.get(..label.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(label))
                && line.get(label.len()..).is_some_and(|value| {
                    value.starts_with(' ') && value.bytes().any(|byte| byte.is_ascii_digit())
                })
        }) {
            readings.push(line.to_owned());
        }
    }
    (readings
        .iter()
        .any(|line| line.eq_ignore_ascii_case("OFFENSE"))
        && readings
            .iter()
            .any(|line| line.eq_ignore_ascii_case("DEFENSE"))
        && readings
            .iter()
            .any(|line| line.bytes().any(|byte| byte.is_ascii_digit())))
    .then(|| readings.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_gear_bonuses_and_requires_the_stats_panel() {
        let text = "Equipment\nDamage +21%\nStats\nRUN\nGold 2\nOFFENSE\nDamage +27%\nDEFENSE\nHealth 72 / 353\nUTILITY\nLuck +35%";
        let report = panel_text(text).unwrap();
        assert!(report.contains("Damage +27%"));
        assert!(!report.contains("Damage +21%"));
        assert!(report.contains("Health 72 / 353"));
        assert!(panel_text("Equipment\nDamage +21%").is_none());
    }
}

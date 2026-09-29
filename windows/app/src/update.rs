//! Обновления Windows-версии через GitHub Releases (на Маке — Sparkle).
//!
//! Соглашение о релизах: тег `win-v<версия>` (например, `win-v0.4.11`, бета — `win-v0.5.0-beta.1`
//! и отметка prerelease), в релизе `keyboop-windows-x64.zip` с keyboop.exe внутри и рядом
//! `keyboop-windows-x64.zip.sha256`. Архив без совпавшего хеша не устанавливается.
//!
//! Здесь только выбор версии (проверяется тестами); скачивание и подмена exe — в `win::update`.

use serde::Deserialize;

pub const REPO: &str = "krazzer00/keyboop";
pub const ASSET: &str = "keyboop-windows-x64.zip";
pub const TAG_PREFIX: &str = "win-v";

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// Версия для сравнения: числа ядра и признак предварительной версии (она младше выпуска с теми
/// же числами).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    core: Vec<u64>,
    /// true — выпуск; false — пре-релиз (меньше выпуска).
    release: bool,
    pre: Vec<u64>,
}

pub fn parse_version(s: &str) -> Option<Version> {
    let s = s
        .trim()
        .trim_start_matches(TAG_PREFIX)
        .trim_start_matches('v');
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (s, None),
    };
    let core: Vec<u64> = core
        .split('.')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    if core.is_empty() {
        return None;
    }
    let pre_nums = pre
        .map(|p| p.split(['.', '-']).filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_default();
    Some(Version {
        core,
        release: pre.is_none(),
        pre: pre_nums,
    })
}

pub fn current() -> Version {
    parse_version(env!("CARGO_PKG_VERSION")).expect("версия пакета")
}

/// Самый новый подходящий релиз новее текущей версии.
pub fn newest<'a>(releases: &'a [Release], current: &Version, beta: bool) -> Option<&'a Release> {
    releases
        .iter()
        .filter(|r| !r.draft && r.tag_name.starts_with(TAG_PREFIX))
        .filter(|r| beta || !r.prerelease)
        .filter(|r| r.assets.iter().any(|a| a.name == ASSET))
        .filter_map(|r| parse_version(&r.tag_name).map(|v| (v, r)))
        .filter(|(v, _)| v > current)
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r)| r)
}

/// Список релизов из GitHub.
pub fn fetch() -> Result<Vec<Release>, String> {
    // Адрес можно подменить для проверки установки на тестовом «релизе».
    let url = std::env::var("KEYBOOP_UPDATE_URL")
        .unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases?per_page=30"));
    let body = ureq::get(&url)
        .header(
            "User-Agent",
            concat!("Keyboop-Windows/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?
        .into_body()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

/// Строка из файла `.sha256` («хеш  имя» или просто хеш).
pub fn parse_sha(text: &str) -> Option<String> {
    let h = text.split_whitespace().next()?.to_ascii_lowercase();
    (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())).then_some(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str, pre: bool) -> Release {
        Release {
            tag_name: tag.into(),
            prerelease: pre,
            draft: false,
            html_url: String::new(),
            assets: vec![Asset {
                name: ASSET.into(),
                browser_download_url: format!("https://example/{tag}.zip"),
            }],
        }
    }

    #[test]
    fn versions_compare() {
        let v = |s| parse_version(s).unwrap();
        assert!(v("win-v0.4.11") > v("0.4.10"));
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("0.5.0") > v("0.5.0-beta.2"));
        assert!(v("0.5.0-beta.2") > v("0.5.0-beta.1"));
        assert!(v("0.5.0-beta.1") > v("0.4.10"));
        assert!(parse_version("win-vX").is_none());
    }

    #[test]
    fn picks_newest_for_channel() {
        let list = vec![
            rel("win-v0.4.9", false),
            rel("win-v0.4.12", false),
            rel("win-v0.5.0-beta.1", true),
            rel("v1.0.0", false), // мак-релиз: не наш тег
        ];
        let cur = parse_version("0.4.10").unwrap();
        assert_eq!(newest(&list, &cur, false).unwrap().tag_name, "win-v0.4.12");
        assert_eq!(
            newest(&list, &cur, true).unwrap().tag_name,
            "win-v0.5.0-beta.1"
        );
        let latest = parse_version("0.5.0").unwrap();
        assert!(newest(&list, &latest, true).is_none());
    }

    #[test]
    fn sha_file() {
        let h = "a".repeat(64);
        assert_eq!(
            parse_sha(&format!("{h}  keyboop-windows-x64.zip\n")),
            Some(h.clone())
        );
        assert_eq!(parse_sha("nope"), None);
    }
}

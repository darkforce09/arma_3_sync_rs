//! Release version comparison and syntax.

use std::cmp::Ordering;

/// Whether `candidate` is a newer version than `current`.
///
/// Both are trimmed and split on `.`; each component keeps its leading ASCII digits (empty or
/// above `i32::MAX` counts as 0) and missing components count as 0. So `2026.2.6-beta` equals
/// `2026.2.6`, and `0.1.0-alpha.1` reads as `0.1.0.1`.
// Mirrors a3sUpdater/service/VersionComparator.java:7-24
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let left = components(candidate);
    let right = components(current);
    let count = left.len().max(right.len());
    for index in 0..count {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        match a.cmp(&b) {
            Ordering::Equal => {}
            order => return order == Ordering::Greater,
        }
    }
    false
}

fn components(version: &str) -> Vec<u32> {
    version
        .trim()
        .split('.')
        .map(|part| {
            let digits_end = part
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(part.len());
            part[..digits_end]
                .parse::<i32>()
                .ok()
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0)
        })
        .collect()
}

/// Whether `version` (trimmed) is a supported release version.
///
/// Three or four dot-separated ASCII numbers, optionally followed by `-` or `+` and dot- or
/// dash-separated alphanumeric labels (`2026.2.1`, `2026.2.1.4`, `1.2.3-rc.1`).
// Mirrors a3sUpdater/dao/JsonManifestDAO.java:14-15,96-98
pub fn is_supported_version(version: &str) -> bool {
    let version = version.trim();
    let (numbers, suffix) = match version.find(['-', '+']) {
        Some(index) => (&version[..index], Some(&version[index + 1..])),
        None => (version, None),
    };
    let parts: Vec<&str> = numbers.split('.').collect();
    let numeric = (3..=4).contains(&parts.len())
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
    numeric && suffix.is_none_or(is_label_list)
}

/// `[A-Za-z0-9]+([.-][A-Za-z0-9]+)*`
fn is_label_list(text: &str) -> bool {
    text.split(['.', '-'])
        .all(|label| !label.is_empty() && label.bytes().all(|b| b.is_ascii_alphanumeric()))
}

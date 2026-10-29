//! Update check against this fork's GitHub Releases (the repository named in
//! `bug_report::REPO_URL`): one request a day when enabled. The result is a
//! toast with an Update button; `update_install` then downloads that release's
//! disk image, swaps it for the app that is running and opens it again, so the
//! download never has to be dragged into place by hand.
//!
//! Test builds are published as pre-releases tagged `vX.Y.Z-test.BUILD`, so
//! the check lists releases (the `/releases/latest` endpoint skips
//! pre-releases) and compares versions by semver, pre-release identifiers
//! included: `0.1.0-test.159` is newer than `0.1.0-test.99` and older than
//! `0.1.0`. Someone running a test build is offered newer test builds even
//! with "Include beta releases" off.

use std::sync::mpsc::{channel, Receiver};

use crate::bug_report::REPO_URL;

/// `owner/name` of the repository whose releases are checked, taken from
/// `REPO_URL` so bug reports and updates always point at the same place.
pub fn repo() -> &'static str {
    REPO_URL
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")
        .unwrap_or(REPO_URL)
}

/// The version this build calls itself for the update check. Builds made by
/// `scripts/bundle.sh` / `scripts/dmg.sh` carry `APATURE_BUILD` and are test
/// builds, so they are `X.Y.Z-test.BUILD`, the same shape as the release tag.
/// A plain `cargo run` has no build number and is just `X.Y.Z`.
///
/// `APATURE_UPDATE_BUILD` (read at run time) pretends to be another build
/// number, for testing the check without rebuilding.
pub fn current_version() -> String {
    let fake = std::env::var("APATURE_UPDATE_BUILD").ok();
    version_for(
        env!("CARGO_PKG_VERSION"),
        fake.as_deref().or(option_env!("APATURE_BUILD")),
    )
}

fn version_for(pkg: &str, build: Option<&str>) -> String {
    match build
        .map(str::trim)
        .filter(|b| !b.is_empty() && b.chars().all(|c| c.is_ascii_digit()))
    {
        Some(b) => format!("{pkg}-test.{b}"),
        None => pkg.to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub prerelease: bool,
    pub page: String,
    /// Direct link to the .dmg asset, when the release has one (the
    /// `-universal.dmg` one when there is a choice).
    pub dmg: Option<String>,
    /// The SHA-256 of that disk image, as the release notes publish it.
    /// `None` when the release does not say, which the installer refuses:
    /// a download that cannot be checked against what was published is not
    /// installed.
    pub sha256: Option<String>,
    pub notes: String,
}

/// The published digest of a release's disk image, from its notes.
///
/// `release.yml` writes one line, `**SHA-256** of the disk image: `…``, so
/// the digest is the first 64-character hex run on a line that names
/// SHA-256. Releases built before that line existed have none, and an empty
/// answer has to mean "uncheckable", never "checked".
pub fn digest_from_notes(notes: &str) -> Option<String> {
    for line in notes.lines() {
        if !line.contains("SHA-256") {
            continue;
        }
        let bytes = line.as_bytes();
        let mut run = 0usize;
        for (i, b) in bytes.iter().enumerate() {
            if !b.is_ascii_hexdigit() {
                run = 0;
                continue;
            }
            run += 1;
            if run == 64 {
                // A longer run is not a digest; keep looking past it.
                let ends = match bytes.get(i + 1) {
                    Some(next) => !next.is_ascii_hexdigit(),
                    None => true,
                };
                if ends {
                    return Some(line[i + 1 - 64..=i].to_ascii_lowercase());
                }
                run = 0;
            }
        }
    }
    None
}

/// Parsed `X.Y.Z[-pre][+meta]`.
#[derive(Debug, PartialEq, Eq)]
struct Version {
    nums: Vec<u64>,
    pre: Vec<String>,
}

fn parse(v: &str) -> Option<Version> {
    let v = v.trim().trim_start_matches('v');
    let v = v.split_once('+').map_or(v, |(a, _)| a);
    let (num, pre) = match v.split_once('-') {
        Some((n, p)) => (n, p),
        None => (v, ""),
    };
    let mut nums = num
        .split('.')
        .map(|x| x.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    while nums.len() < 3 {
        nums.push(0);
    }
    let pre = if pre.is_empty() {
        Vec::new()
    } else {
        pre.split('.').map(str::to_string).collect()
    };
    Some(Version { nums, pre })
}

/// Semver precedence of two pre-release identifier lists (both non-empty):
/// numeric identifiers compare as numbers and sort below alphanumeric ones,
/// and a shorter list sorts first when it is a prefix of the longer.
fn cmp_pre(a: &[String], b: &[String]) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    for (x, y) in a.iter().zip(b) {
        let o = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(m), Ok(n)) => m.cmp(&n),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.cmp(y),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    a.len().cmp(&b.len())
}

fn cmp(a: &Version, b: &Version) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    a.nums
        .cmp(&b.nums)
        .then_with(|| match (a.pre.is_empty(), b.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => cmp_pre(&a.pre, &b.pre),
        })
}

/// Semver compare of "1.2.3" / "v1.2.3-test.159"; a pre-release sorts below
/// its stable. Returns true when `a` is newer than `b`; false when either
/// does not parse.
pub fn is_newer(a: &str, b: &str) -> bool {
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => cmp(&a, &b) == std::cmp::Ordering::Greater,
        _ => false,
    }
}

/// From a GitHub "list releases" response, the newest release that is newer
/// than `current`, skipping drafts. Pre-releases count when `beta` is on or
/// when `current` is itself a pre-release (a test build).
pub fn pick(releases: &serde_json::Value, current: &str, beta: bool) -> Option<Update> {
    let want_pre = beta || parse(current).is_some_and(|v| !v.pre.is_empty());
    let mut best: Option<Update> = None;
    for r in releases.as_array().map(Vec::as_slice).unwrap_or_default() {
        if r.get("draft").and_then(|d| d.as_bool()).unwrap_or(false) {
            continue;
        }
        let pre = r
            .get("prerelease")
            .and_then(|p| p.as_bool())
            .unwrap_or(false);
        if pre && !want_pre {
            continue;
        }
        let tag = r
            .get("tag_name")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim_start_matches('v')
            .to_string();
        if tag.is_empty() || !is_newer(&tag, current) {
            continue;
        }
        if best.as_ref().is_some_and(|b| !is_newer(&tag, &b.version)) {
            continue;
        }
        let str_of = |v: &serde_json::Value, k: &str| {
            v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
        };
        let assets = r
            .get("assets")
            .and_then(|a| a.as_array())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let dmg_named = |suffix: &str| {
            assets.iter().find_map(|x| {
                let n = x.get("name")?.as_str()?;
                if n.ends_with(suffix) {
                    x.get("browser_download_url")?.as_str().map(str::to_string)
                } else {
                    None
                }
            })
        };
        let notes = str_of(r, "body");
        best = Some(Update {
            version: tag,
            prerelease: pre,
            page: str_of(r, "html_url"),
            dmg: dmg_named("-universal.dmg").or_else(|| dmg_named(".dmg")),
            sha256: digest_from_notes(&notes),
            notes,
        });
    }
    best
}

/// Fetch the newest release that is newer than `current`; `beta` includes
/// pre-releases. Blocking; call from a thread.
pub fn check(current: &str, beta: bool) -> Result<Option<Update>, String> {
    // APATURE_UPDATE_REPO lets a test point at another public repository.
    let repo = std::env::var("APATURE_UPDATE_REPO").unwrap_or_else(|_| repo().to_string());
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=30");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .into();
    let body: serde_json::Value = agent
        .get(&url)
        .header("User-Agent", &format!("Apature/{current}"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => {
                "no releases visible (a private repository hides them)".to_string()
            }
            other => other.to_string(),
        })?
        .body_mut()
        .read_json()
        .map_err(|e| e.to_string())?;
    Ok(pick(&body, current, beta))
}

pub struct Checker {
    rx: Receiver<Result<Option<Update>, String>>,
}

impl Checker {
    pub fn spawn(current: String, beta: bool, ctx: egui::Context) -> Checker {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let _ = tx.send(check(&current, beta));
                ctx.request_repaint();
            })
            .expect("spawn update check");
        Checker { rx }
    }
    pub fn try_recv(&self) -> Option<Result<Option<Update>, String>> {
        self.rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> serde_json::Value {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(&path).expect(&path)).expect("fixture json")
    }

    #[test]
    fn repo_comes_from_bug_report_url() {
        assert_eq!(repo(), "Salcedoandrew/salcedos-apature");
        assert_eq!(format!("https://github.com/{}", repo()), REPO_URL);
    }

    #[test]
    fn current_version_shape() {
        assert_eq!(version_for("0.1.0", Some("159")), "0.1.0-test.159");
        assert_eq!(version_for("0.1.0", Some(" 7 ")), "0.1.0-test.7");
        assert_eq!(version_for("0.1.0", None), "0.1.0");
        assert_eq!(version_for("0.1.0", Some("dev")), "0.1.0");
        assert_eq!(version_for("0.1.0", Some("")), "0.1.0");
    }

    #[test]
    fn version_order() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(is_newer("0.2.0", "0.2.0-beta.1"));
        assert!(!is_newer("0.2.0-beta.1", "0.2.0"));
        assert!(is_newer("0.2.0-beta.2", "0.2.0-beta.1"));
        assert!(!is_newer("0.1.9", "0.10.0"));
        assert!(is_newer("0.1", "0.0.9"));
        assert!(!is_newer("0.1", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("0.1.0", "nightly"));
        assert!(!is_newer("0.1.0+abc", "0.1.0"));
    }

    #[test]
    fn test_build_numbers_compare_as_numbers() {
        assert!(is_newer("v0.1.0-test.159", "0.1.0-test.99"));
        assert!(!is_newer("v0.1.0-test.99", "0.1.0-test.159"));
        assert!(is_newer("v0.1.0-test.160", "0.1.0-test.159"));
        assert!(!is_newer("v0.1.0-test.159", "0.1.0-test.159"));
        assert!(!is_newer("v0.1.0-test.158", "0.1.0-test.159"));
        // A stable is newer than any test build of it, older than the next.
        assert!(is_newer("0.1.0", "0.1.0-test.159"));
        assert!(is_newer("0.1.1-test.1", "0.1.0"));
        assert!(!is_newer("0.1.0-test.999", "0.1.0"));
        // Semver: numeric identifiers sort below alphanumeric, shorter first.
        assert!(is_newer("1.0.0-alpha.beta", "1.0.0-alpha.1"));
        assert!(is_newer("1.0.0-alpha.1", "1.0.0-alpha"));
        assert!(is_newer("1.0.0-beta", "1.0.0-alpha.beta"));
    }

    #[test]
    fn live_shape_fixture() {
        let rel = fixture("releases_test159.json");
        // A test build at 159 is up to date; the same tag is never offered.
        assert_eq!(pick(&rel, "0.1.0-test.159", false), None);
        assert_eq!(pick(&rel, "0.1.0-test.200", false), None);
        // An older test build is offered 159 even with betas off.
        let u = pick(&rel, "0.1.0-test.100", false).expect("offer 159");
        assert_eq!(u.version, "0.1.0-test.159");
        assert!(u.prerelease);
        assert_eq!(
            u.page,
            "https://github.com/Salcedoandrew/salcedos-apature/releases/tag/v0.1.0-test.159"
        );
        assert_eq!(
            u.dmg.as_deref(),
            Some("https://github.com/Salcedoandrew/salcedos-apature/releases/download/v0.1.0-test.159/Apature-0.1.0-test-159-universal.dmg")
        );
        // A stable build only sees pre-releases with betas on, and 0.1.0
        // stable is newer than its own test builds.
        assert_eq!(pick(&rel, "0.0.9", false), None);
        assert_eq!(
            pick(&rel, "0.0.9", true).map(|u| u.version),
            Some("0.1.0-test.159".into())
        );
        assert_eq!(pick(&rel, "0.1.0", true), None);
    }

    #[test]
    fn mixed_fixture_picks_newest_suitable() {
        let rel = fixture("releases_mixed.json");
        // Newest non-draft wins regardless of list order; the draft 0.2.0
        // and the unparseable "nightly" tag are skipped.
        for cur in ["0.1.0-test.1", "0.1.0-test.99", "0.1.0-test.159"] {
            let u = pick(&rel, cur, false).expect(cur);
            assert_eq!(u.version, "0.1.0-test.160", "from {cur}");
            assert_eq!(
                u.dmg.as_deref(),
                Some("https://github.com/Salcedoandrew/salcedos-apature/releases/download/v0.1.0-test.160/Apature-0.1.0-test-160-universal.dmg")
            );
            assert_eq!(u.notes, "Notes for v0.1.0-test.160");
        }
        assert_eq!(pick(&rel, "0.1.0-test.160", false), None);
        assert_eq!(pick(&rel, "0.1.0-test.161", true), None);
        // Stable users without betas get only the stable 0.0.9, if newer.
        assert_eq!(
            pick(&rel, "0.0.8", false).map(|u| (u.version, u.dmg)),
            Some((
                "0.0.9".into(),
                Some("https://github.com/Salcedoandrew/salcedos-apature/releases/download/v0.0.9/Apature-0.0.9.dmg".into())
            ))
        );
        assert_eq!(pick(&rel, "0.0.9", false), None);
        assert_eq!(pick(&rel, "0.1.0", true), None);
        assert_eq!(pick(&serde_json::json!([]), "0.1.0-test.1", true), None);
        assert_eq!(
            pick(&serde_json::json!({"message": "Not Found"}), "0.1.0", true),
            None
        );
    }

    #[test]
    fn the_published_digest_is_read_from_the_notes() {
        let body = "## [0.1.4] — 2026-10-28\n\n### Added\n- Warmer.\n\n\n**SHA-256** of the disk image: `B6CE6F88851AB23FBFABEA24EFF48A4786486EFEBA1D2CF738189096612DE0B8`\n\n> This build is ad-hoc signed.\n";
        // Uppercase in the notes is the same digest, answered in lower case.
        assert_eq!(
            digest_from_notes(body).as_deref(),
            Some("b6ce6f88851ab23fbfabea24eff48a4786486efeba1d2cf738189096612de0b8")
        );
        // Nothing to check is None, never a guess.
        assert_eq!(digest_from_notes("SHA-256: `b6ce6f88`"), None);
        assert_eq!(digest_from_notes("no digest here"), None);
        assert_eq!(digest_from_notes(""), None);
        // A run that is part of a longer one is not a digest.
        let long = format!("SHA-256: {}aa", "b".repeat(64));
        assert_eq!(digest_from_notes(&long), None);
    }

    #[test]
    fn an_offer_carries_the_digest_of_its_image() {
        let digest = "ab".repeat(32);
        let asset = serde_json::json!({
            "name": "Apature-9.9.9.dmg",
            "browser_download_url": "https://example.com/a.dmg"
        });
        let with = serde_json::json!([{
            "tag_name": "v9.9.9",
            "prerelease": false,
            "draft": false,
            "html_url": "https://example.com/r",
            "body": format!("**SHA-256** of the disk image: `{digest}`"),
            "assets": [asset.clone()]
        }]);
        let offer = pick(&with, "0.1.4", false).expect("offer");
        assert_eq!(offer.sha256.as_deref(), Some(digest.as_str()));

        let without = serde_json::json!([{
            "tag_name": "v9.9.9",
            "prerelease": false,
            "draft": false,
            "html_url": "https://example.com/r",
            "body": "Notes with no digest",
            "assets": [asset]
        }]);
        let offer = pick(&without, "0.1.4", false).expect("offer");
        assert_eq!(offer.sha256, None);
    }

    /// Live GitHub API: `cargo test -p apature -- --ignored live_check`.
    #[test]
    #[ignore = "network"]
    fn live_check() {
        let up = check("0.1.0-test.159", false).expect("check");
        eprintln!("as 0.1.0-test.159: {up:?}");
        assert!(up
            .as_ref()
            .map_or(true, |u| is_newer(&u.version, "0.1.0-test.159")));
        let old = check("0.1.0-test.100", false)
            .expect("check")
            .expect("offer");
        eprintln!("as 0.1.0-test.100: {} {:?}", old.version, old.dmg);
        assert!(!is_newer("0.1.0-test.159", &old.version));
        assert!(old
            .dmg
            .as_deref()
            .is_some_and(|d| d.ends_with("-universal.dmg")));
    }
}

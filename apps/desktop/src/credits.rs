//! Who made this build: Apature was created by McCal-Codes (Caleb McCartney);
//! this edition adds Andrew Salcedo's changes on top. About Apature and the
//! README's "About this fork" section show the same list (a test keeps them
//! in step). The upstream copyright and licences stay as they are.

/// The original project.
pub const UPSTREAM_NAME: &str = "McCal-Codes";
pub const UPSTREAM_URL: &str = "https://github.com/McCal-Codes/apature";

/// This edition (Help › Apature on GitHub, bug reports).
pub const FORK_AUTHOR: &str = "Andrew Salcedo";
pub const FORK_URL: &str = crate::bug_report::REPO_URL;

/// Andrew Salcedo's additions, in plain words.
pub const FORK_CHANGES: &[&str] = &[
    "Photo Mechanic keyboard shortcuts",
    "RAW preview extraction for all major cameras",
    "Select By rating, color class and tag",
    "Copy Photos",
    "Extract JPEG Previews from RAW",
    "Sidebar folder tree",
    "G closes Preview",
    "Ratings saved to XMP sidecars (originals untouched)",
    "Rating and preview fixes",
    "Help › Report a Bug",
];

#[cfg(test)]
mod tests {
    use super::*;

    const README: &str = include_str!("../../../README.md");

    #[test]
    fn credits_name_both_projects() {
        assert_ne!(UPSTREAM_URL, FORK_URL);
        assert!(UPSTREAM_URL.ends_with("/McCal-Codes/apature"));
        assert!(FORK_URL.ends_with("/Salcedoandrew/salcedos-apature"));
        assert_eq!(FORK_URL, crate::bug_report::REPO_URL);
        assert!(!FORK_CHANGES.is_empty());
    }

    #[test]
    fn readme_credits_match_the_about_window() {
        let section = README
            .split("## About this fork")
            .nth(1)
            .and_then(|s| s.split("\n## ").next())
            .expect("README has an About this fork section");
        assert!(section.contains(UPSTREAM_URL) && section.contains(UPSTREAM_NAME));
        assert!(section.contains(FORK_URL) && section.contains(FORK_AUTHOR));
        for c in FORK_CHANGES {
            assert!(section.contains(c), "README credits miss {c:?}");
        }
        // The upstream licence and copyright stay.
        assert!(README.contains("Caleb McCartney"));
        assert!(README.contains("MIT OR Apache-2.0"));
    }
}

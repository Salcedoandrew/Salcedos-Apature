//! Where the app keeps its files on disk. One definition of the
//! `ProjectDirs` identity, shared by the CLI and the desktop app so the two
//! can never drift into different data, cache or config directories.

/// The project directories both apps use (data dir for the ingest ledger,
/// cache dir for thumbnails, config dir for preferences). `None` when the
/// platform has no home directory to anchor them to.
pub fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("dev", "mccal", "Apature")
}

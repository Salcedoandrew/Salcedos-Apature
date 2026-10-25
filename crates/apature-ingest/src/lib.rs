//! Card ingest. The rules, which are non-negotiable:
//!
//! 1. **Plan first.** Every destination path is computed and checked for
//!    collisions (on disk and within the plan) before a single byte moves.
//!    A plan with collisions cannot run.
//! 2. **Copies are verified.** Temp name, full SHA-256 of source and copy,
//!    then rename. Secondary destinations are verified the same way.
//! 3. **Sources are never deleted.** Erasing a card is the photographer's
//!    job, in camera, after the report says every file verified everywhere.
//! 4. **Sequential.** A card reader is the bottleneck; parallel copies are
//!    slower and fragment the card.
//!
//! The module split: [`template`] expands naming templates, [`plan`] builds
//! the plan, [`run`] executes it with progress events and cancellation,
//! [`ledger`] remembers what has been ingested for "only new photos",
//! [`copy`] is the verified copy primitive, and [`photocopy`] is Copy
//! Photos… from the contact sheet on top of it.

pub mod copy;
pub mod ledger;
pub mod photocopy;
pub mod plan;
pub mod run;
pub mod template;

pub use copy::{copy_unverified, copy_verified, sha256_file, CopyReport, IngestError};
pub use ledger::Ledger;
pub use photocopy::{
    dest_base, plan_copy, run_copy, CopyOptions, CopyOutcome, CopyPlan, CopyProgress, DateFolders,
    DestMode, OnCollision,
};
pub use plan::{
    build_plan, example_names, today, Collision, CollisionPolicy, Entry, FileFilter, Options, Plan,
    Skip,
};
pub use run::{run, Event, FileOutcome, Report};
pub use template::Facts;

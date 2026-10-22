//! Every user action is a `Command`, whether it came from the menu bar, a
//! key, a toolbar button or a sidebar row. One dispatcher, one keymap.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workspace {
    Sheet,
    Preview,
    Caption,
    Ingest,
    Export,
}

impl Workspace {
    pub fn label(self) -> &'static str {
        match self {
            Workspace::Sheet => "Sheet",
            Workspace::Preview => "Preview",
            Workspace::Caption => "Caption",
            Workspace::Ingest => "Ingest",
            Workspace::Export => "Export",
        }
    }
    /// Workspaces that exist today. The others are shown disabled with the
    /// milestone that brings them, never as controls that do nothing.
    pub fn available(self) -> Result<(), &'static str> {
        match self {
            Workspace::Sheet
            | Workspace::Preview
            | Workspace::Caption
            | Workspace::Ingest
            | Workspace::Export => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    OpenFolder(PathBuf),
    /// Click on a cell with modifiers.
    Click {
        idx: usize,
        shift: bool,
        command: bool,
    },
    /// Keyboard focus move by `delta` positions in the view, with modifiers.
    MoveFocus {
        delta: isize,
        shift: bool,
        command: bool,
    },
    SelectAll,
    SelectNone,
    Undo,
    Redo,
    SetFilter(crate::filter::Filter),
    ClearFilter(Option<crate::filter::Clause>),
    SetSort(crate::filter::Sort),
    ToggleSortReversed,
    ToggleAdvanceAfterMark,
    LoadAssignment,
    LoadCodes,
    ApplyStationery,
    InsertCaptionTemplate,
    SaveCaption,
    SaveCaptionAndNext,
    /// Write the changed fields to every selected photo (Photo Mechanic's "apply to all selected").
    SaveCaptionToSelection,
    RevertCaption,
    IngestChooseSource,
    IngestChoosePrimary,
    IngestChooseSecondary,
    IngestStart,
    IngestCancel,
    IngestOpenResult,
    /// Unmount the ingested card (after a verified ingest).
    IngestEject,
    /// Live Ingest: watch the source folder and ingest what arrives.
    LiveToggle,
    /// Bring the main window to the front (menu bar extra).
    ShowMainWindow,
    /// Window menu items of the Photo Mechanic layout (⌘⇧W; Minimize has no key).
    CloseWindow,
    MinimizeWindow,
    /// Help menu: Settings > Shortcuts, the log, the repository.
    OpenShortcuts,
    OpenAbout,
    /// Updates: check now (menu), open the newest release's DMG.
    CheckUpdates,
    OpenUpdate,
    RevealLog,
    OpenRepo,
    /// Copy the focused photo's marks and IPTC; paste onto the selection.
    CopyMetadata,
    PasteMetadata,
    ExportChooseDest,
    ExportStart,
    ExportCancel,
    ExportOpenResult,
    /// Preview: toggle 1:1 and fit.
    ToggleZoom,
    /// File ops: open the Rename dialog, apply it, open the Trash dialog, confirm it.
    RenamePhotos,
    RenameApply,
    MoveToTrash,
    TrashConfirm,
    CloseDialog,
    /// Preview: compare the selection side by side.
    ToggleCompare,
    CloseTab,
    NextTab,
    PrevTab,
    SelectTab(usize),
    OpenSettings,
    ClearThumbCache,
    PickFolder,
    AddFavorite,
    RemoveFavorite(PathBuf),
    RevealInFinder,
    ToggleSidebar,
    ToggleInspector,
    ToggleFilmstrip,
    /// Rescan the open folder, keeping the cursor and selection by path.
    RefreshFolder,
    /// Preview-only brightness: 0, +1, +2 stops (B key).
    CyclePreviewBoost,
    /// Crop tool in Preview (K): toggle, apply (⏎), cancel (Esc), clear (⌫),
    /// next ratio (R), next overlay (G).
    CropToggle,
    /// Loupe under the pointer in Preview (L).
    ToggleLoupe,
    /// Expand or collapse the focused burst stack (S).
    ToggleStack,
    CropApply,
    CropCancel,
    CropClear,
    CropNextRatio,
    CropNextOverlay,
    /// Send the files from the last export to the chosen target.
    DeliverLast,
    /// Connect and log in to a target without sending (Settings > Delivery).
    DeliverTest(String),
    /// Send again only the files that failed in the last delivery.
    DeliverResendFailed,
    SetWorkspace(Workspace),
    ZoomIn,
    ZoomOut,
    Move(isize),
    MovePage(isize),
    Home,
    End,
    SetRating(i8),
    SetClass(u8),
    ToggleTag,
    /// A quarter turn, losslessly, in the sidecar (`true` = clockwise).
    Rotate(bool),
    Escape,
    Activate,
}

impl Command {
    /// Menu identifiers are strings so the native menu can round-trip them.
    pub fn menu_id(&self) -> String {
        match self {
            Command::PickFolder => "file.open".into(),
            Command::AddFavorite => "file.favorite".into(),
            Command::RevealInFinder => "file.reveal".into(),
            Command::ToggleSidebar => "view.sidebar".into(),
            Command::ToggleInspector => "view.inspector".into(),
            Command::ToggleFilmstrip => "view.filmstrip".into(),
            Command::RefreshFolder => "view.refresh".into(),
            Command::CyclePreviewBoost => "view.boost".into(),
            Command::CropToggle => "image.crop".into(),
            Command::ToggleLoupe => "view.loupe".into(),
            Command::ToggleStack => "view.stack".into(),
            Command::CropApply => "image.crop.apply".into(),
            Command::CropCancel => "image.crop.cancel".into(),
            Command::CropClear => "image.crop.clear".into(),
            Command::CropNextRatio => "image.crop.ratio".into(),
            Command::CropNextOverlay => "image.crop.overlay".into(),
            Command::DeliverLast => "file.deliver".into(),
            Command::DeliverTest(n) => format!("delivery.test.{n}"),
            Command::DeliverResendFailed => "file.deliver.resend".into(),
            Command::SetWorkspace(w) => format!("view.workspace.{}", w.label().to_lowercase()),
            Command::SelectAll => "edit.selectall".into(),
            Command::SelectNone => "edit.selectnone".into(),
            Command::Undo => "edit.undo".into(),
            Command::Redo => "edit.redo".into(),
            Command::ClearFilter(None) => "view.clearfilter".into(),
            Command::SetSort(s) => format!("view.sort.{}", s.label().replace(' ', "")),
            Command::ToggleSortReversed => "view.sortreversed".into(),
            Command::ToggleAdvanceAfterMark => "image.advance".into(),
            Command::LoadAssignment => "file.assignment".into(),
            Command::LoadCodes => "file.codes".into(),
            Command::ApplyStationery => "image.stationery".into(),
            Command::SaveCaption => "image.savecaption".into(),
            Command::SaveCaptionToSelection => "image.savecaption.selection".into(),
            Command::IngestStart => "file.ingest".into(),
            Command::CopyMetadata => "edit.copymeta".into(),
            Command::PasteMetadata => "edit.pastemeta".into(),
            Command::ExportStart => "file.export".into(),
            Command::RenamePhotos => "file.rename".into(),
            Command::MoveToTrash => "file.trash".into(),
            Command::CloseTab => "file.closetab".into(),
            Command::NextTab => "window.nexttab".into(),
            Command::PrevTab => "window.prevtab".into(),
            Command::OpenSettings => "app.settings".into(),
            Command::ZoomIn => "view.zoomin".into(),
            Command::ZoomOut => "view.zoomout".into(),
            Command::ToggleZoom => "view.zoom11".into(),
            Command::ToggleCompare => "view.compare".into(),
            Command::SetRating(r) => format!("image.rating.{r}"),
            Command::SetClass(c) => format!("image.class.{c}"),
            Command::ToggleTag => "image.tag".into(),
            Command::Rotate(true) => "image.rotate.cw".into(),
            Command::Rotate(false) => "image.rotate.ccw".into(),
            Command::ShowMainWindow => "app.show".into(),
            Command::CloseWindow => "window.close".into(),
            Command::MinimizeWindow => "window.minimize".into(),
            Command::LiveToggle => "ingest.live".into(),
            Command::OpenShortcuts => "help.shortcuts".into(),
            Command::OpenAbout => "app.about".into(),
            Command::CheckUpdates => "app.updates".into(),
            Command::RevealLog => "help.log".into(),
            Command::OpenRepo => "help.repo".into(),
            other => format!("other.{other:?}"),
        }
    }

    pub fn from_menu_id(id: &str) -> Option<Command> {
        Some(match id {
            "file.open" => Command::PickFolder,
            "file.favorite" => Command::AddFavorite,
            "file.reveal" => Command::RevealInFinder,
            "view.sidebar" => Command::ToggleSidebar,
            "view.inspector" => Command::ToggleInspector,
            "view.filmstrip" => Command::ToggleFilmstrip,
            "view.refresh" => Command::RefreshFolder,
            "view.boost" => Command::CyclePreviewBoost,
            "image.crop" => Command::CropToggle,
            "view.loupe" => Command::ToggleLoupe,
            "ingest.live" => Command::LiveToggle,
            "app.show" => Command::ShowMainWindow,
            "window.close" => Command::CloseWindow,
            "window.minimize" => Command::MinimizeWindow,
            "help.shortcuts" => Command::OpenShortcuts,
            "app.about" => Command::OpenAbout,
            "app.updates" => Command::CheckUpdates,
            "help.log" => Command::RevealLog,
            "help.repo" => Command::OpenRepo,
            "view.stack" => Command::ToggleStack,
            "image.crop.clear" => Command::CropClear,
            "file.deliver" => Command::DeliverLast,
            "file.deliver.resend" => Command::DeliverResendFailed,
            "view.workspace.sheet" => Command::SetWorkspace(Workspace::Sheet),
            "view.workspace.preview" => Command::SetWorkspace(Workspace::Preview),
            "view.workspace.caption" => Command::SetWorkspace(Workspace::Caption),
            "view.workspace.ingest" => Command::SetWorkspace(Workspace::Ingest),
            "edit.selectall" => Command::SelectAll,
            "edit.selectnone" => Command::SelectNone,
            "edit.undo" => Command::Undo,
            "edit.redo" => Command::Redo,
            "view.clearfilter" => Command::ClearFilter(None),
            "view.sortreversed" => Command::ToggleSortReversed,
            "image.advance" => Command::ToggleAdvanceAfterMark,
            "file.assignment" => Command::LoadAssignment,
            "file.codes" => Command::LoadCodes,
            "image.stationery" => Command::ApplyStationery,
            "image.savecaption" => Command::SaveCaption,
            "image.savecaption.selection" => Command::SaveCaptionToSelection,
            "file.ingest" => Command::SetWorkspace(Workspace::Ingest),
            "edit.copymeta" => Command::CopyMetadata,
            "edit.pastemeta" => Command::PasteMetadata,
            "file.export" => Command::SetWorkspace(Workspace::Export),
            "file.rename" => Command::RenamePhotos,
            "file.trash" => Command::MoveToTrash,
            "view.workspace.export" => Command::SetWorkspace(Workspace::Export),
            "file.closetab" => Command::CloseTab,
            "window.nexttab" => Command::NextTab,
            "window.prevtab" => Command::PrevTab,
            "app.settings" => Command::OpenSettings,
            "view.zoomin" => Command::ZoomIn,
            "view.zoomout" => Command::ZoomOut,
            "view.zoom11" => Command::ToggleZoom,
            "view.compare" => Command::ToggleCompare,
            "image.tag" => Command::ToggleTag,
            "image.rotate.cw" => Command::Rotate(true),
            "image.rotate.ccw" => Command::Rotate(false),
            _ => {
                if let Some(k) = id.strip_prefix("view.sort.") {
                    Command::SetSort(
                        *crate::filter::Sort::ALL
                            .iter()
                            .find(|s| s.label().replace(' ', "") == k)?,
                    )
                } else if let Some(r) = id.strip_prefix("image.rating.") {
                    Command::SetRating(r.parse().ok()?)
                } else if let Some(c) = id.strip_prefix("image.class.") {
                    Command::SetClass(c.parse().ok()?)
                } else {
                    return None;
                }
            }
        })
    }
}

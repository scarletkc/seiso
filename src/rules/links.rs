use std::path::Path;

use crate::diagnostics::Diagnostic;
use crate::paths::{
    LinkPathError, TargetStatus, local_link_target, local_target_status, normalize,
};

use crate::rules::CheckContext;

#[derive(Default)]
pub(crate) struct LinkResult {
    pub diagnostics: Vec<Diagnostic>,
    pub errors: Vec<String>,
    pub incomplete: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PathStatus {
    Exists,
    Missing,
    Unknown,
    Error(String),
}

pub trait WorkspaceFiles {
    fn status(&self, workspace_root: &Path, target: &Path) -> PathStatus;
}

pub struct LocalWorkspaceFiles;

impl WorkspaceFiles for LocalWorkspaceFiles {
    fn status(&self, workspace_root: &Path, target: &Path) -> PathStatus {
        local_target_status(workspace_root, target).into()
    }
}

impl From<TargetStatus> for PathStatus {
    fn from(status: TargetStatus) -> Self {
        match status {
            TargetStatus::File | TargetStatus::Directory => Self::Exists,
            TargetStatus::Missing => Self::Missing,
            TargetStatus::Unknown | TargetStatus::OutsideWorkspace => Self::Unknown,
            TargetStatus::Unreadable(error) => Self::Error(error),
        }
    }
}

pub(crate) fn check(context: &CheckContext<'_>, files: &dyn WorkspaceFiles) -> LinkResult {
    let mut result = LinkResult::default();
    for link in &context.document.links {
        let destination = link.destination.as_str();
        let target = match local_link_target(context.workspace_root, context.path, destination) {
            Ok(link) => link.path,
            Err(LinkPathError::External) => continue,
            Err(_) => {
                result.incomplete = true;
                continue;
            }
        };
        // The current document can be a new stdin overlay with no disk entry.
        if target == normalize(context.path) {
            continue;
        }
        match files.status(context.workspace_root, &target) {
            PathStatus::Exists => {}
            PathStatus::Missing => {
                result.diagnostics.push(Diagnostic::new(context.filename, &context.document.source,
                    "LNK001", link.span, format!("Local link target {destination:?} does not exist in the workspace."),
                    "Update the path or restore the target; paths resolve from this document's directory, or from the workspace root when they start with /, and seiso does not add .md or index.md."));
            }
            PathStatus::Unknown => {
                result.incomplete = true;
            }
            PathStatus::Error(error) => {
                result.incomplete = true;
                result.errors.push(format!(
                    "Cannot inspect link target {destination:?}: {error}"
                ));
            }
        }
    }
    result.errors.sort();
    result.errors.dedup();
    result
}

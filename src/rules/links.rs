use std::path::Path;

use crate::diagnostics::Diagnostic;
use crate::paths::{
    LinkPathError, TargetStatus, local_link_targets, local_target_status, normalize, select_target,
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

impl From<PathStatus> for TargetStatus {
    fn from(status: PathStatus) -> Self {
        match status {
            PathStatus::Exists => Self::File,
            PathStatus::Missing => Self::Missing,
            PathStatus::Unknown => Self::Unknown,
            PathStatus::Error(error) => Self::Unreadable(error),
        }
    }
}

pub(crate) fn check(context: &CheckContext<'_>, files: &dyn WorkspaceFiles) -> LinkResult {
    let mut result = LinkResult::default();
    let site = context.config.site_routes(context.path);
    let current = normalize(context.path);
    for link in &context.document.links {
        let destination = link.destination.as_str();
        let targets = match local_link_targets(
            context.workspace_root,
            context.path,
            destination,
            site.as_ref(),
        ) {
            Ok(targets) => targets,
            Err(LinkPathError::External) => continue,
            Err(_) => {
                result.incomplete = true;
                continue;
            }
        };
        let (_, status) = select_target(&targets, |target| {
            // The current document can be a new stdin overlay with no disk entry.
            if target.path == current {
                TargetStatus::File
            } else {
                files.status(context.workspace_root, &target.path).into()
            }
        });
        match status {
            TargetStatus::File | TargetStatus::Directory => {}
            TargetStatus::Missing => {
                let suggestion = match &site {
                    None => "Update the path or restore the target; paths resolve from this document's directory, or from the workspace root when they start with /, and seiso does not add .md or index.md.".to_owned(),
                    Some(site) => format!(
                        "Update the path or restore the target; the site rooted at {:?} has no page or file for this route either.",
                        site_root(context.workspace_root, &site.root)
                    ),
                };
                result.diagnostics.push(Diagnostic::new(
                    context.filename,
                    &context.document.source,
                    "LNK001",
                    link.span,
                    format!("Local link target {destination:?} does not exist in the workspace."),
                    suggestion,
                ));
            }
            TargetStatus::Unknown | TargetStatus::OutsideWorkspace => {
                result.incomplete = true;
            }
            TargetStatus::Unreadable(error) => {
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

fn site_root(workspace_root: &Path, root: &Path) -> String {
    match root.strip_prefix(normalize(workspace_root)) {
        Ok(relative) if relative.as_os_str().is_empty() => ".".into(),
        Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
        Err(_) => root.to_string_lossy().into_owned(),
    }
}

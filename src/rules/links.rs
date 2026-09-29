use std::path::Path;

use crate::diagnostics::Diagnostic;
use crate::paths::{
    LinkPathError, Listings, TargetStatus, local_link_targets, local_target_status, normalize,
    select_target,
};

use crate::rules::CheckContext;

#[derive(Default)]
pub(crate) struct LinkResult {
    pub diagnostics: Vec<Diagnostic>,
    pub errors: Vec<String>,
    pub incomplete: bool,
}

pub trait WorkspaceFiles {
    fn status(&self, workspace_root: &Path, target: &Path) -> TargetStatus;
}

#[derive(Default)]
pub struct LocalWorkspaceFiles {
    listings: Listings,
}

impl WorkspaceFiles for LocalWorkspaceFiles {
    fn status(&self, workspace_root: &Path, target: &Path) -> TargetStatus {
        let status = local_target_status(workspace_root, target);
        self.listings.confirm(workspace_root, target, status)
    }
}

pub(crate) fn check(context: &CheckContext<'_>, files: &dyn WorkspaceFiles) -> LinkResult {
    let mut result = LinkResult::default();
    let site = context
        .policy
        .site
        .as_ref()
        .map(|site| context.config.site_routes(site));
    let root = normalize(context.workspace_root);
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
        let (_, status) = select_target(&root, &targets, |target| {
            // The current document can be a new stdin overlay with no disk entry.
            if target.path == current {
                TargetStatus::File
            } else {
                files.status(context.workspace_root, &target.path)
            }
        });
        match status {
            TargetStatus::File | TargetStatus::Directory => {}
            TargetStatus::CaseMismatch(actual) => {
                result.diagnostics.push(Diagnostic::new(
                    context.filename,
                    &context.document.source,
                    "LNK001",
                    link.span,
                    format!(
                        "Local link target {destination:?} differs in letter case from {actual:?}."
                    ),
                    "Change the link to the same letter case; GitHub and Linux treat names that differ only in case as different files, even where Windows or macOS open them.",
                ));
            }
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

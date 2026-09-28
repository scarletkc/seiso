//! Shared lexical paths and workspace-local link targets.

use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

/// Remove lexical `.` and `..` components without accessing the filesystem.
pub fn normalize(path: impl AsRef<Path>) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.as_ref().components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match result.components().next_back() {
                Some(Component::Normal(_)) => {
                    result.pop();
                }
                None | Some(Component::ParentDir) => result.push(".."),
                _ => {}
            },
            _ => result.push(component.as_os_str()),
        }
    }
    result
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum LinkPathError {
    External,
    Template,
    OutsideWorkspace,
    Unknown,
}

/// Where a documentation site serves the pages that a document's links route to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SiteRoutes {
    pub root: PathBuf,
    pub public: Option<PathBuf>,
    /// URL prefix, ending with `/`, that root-relative links include.
    pub base: String,
}

impl SiteRoutes {
    /// The site path of a root-relative link, or `None` for a link outside the base.
    fn route<'a>(&self, path: &'a str) -> Option<&'a str> {
        if path == self.base.trim_end_matches('/') {
            Some("")
        } else {
            path.strip_prefix(self.base.as_str())
        }
    }
}

pub(crate) struct LocalTarget {
    pub path: PathBuf,
    pub target: String,
    /// Reached as a site route rather than as the written repository path.
    pub route: bool,
}

pub(crate) struct LocalLink {
    pub locations: Vec<LocalTarget>,
    pub anchor: Option<String>,
}

/// Resolve a Markdown destination, including whether its anchor can be evaluated.
pub(crate) fn local_link(
    root: &Path,
    source: &Path,
    destination: &str,
    site: Option<&SiteRoutes>,
) -> Result<LocalLink, LinkPathError> {
    let locations = local_link_targets(root, source, destination, site)?;
    if percent_decode(destination).is_some_and(|value| is_template(&value)) {
        return Err(LinkPathError::Template);
    }
    let anchor = destination
        .split_once('#')
        .map(|(_, value)| percent_decode(value).ok_or(LinkPathError::Unknown))
        .transpose()?;
    if anchor.as_deref().is_some_and(is_template) {
        return Err(LinkPathError::Template);
    }
    Ok(LocalLink { locations, anchor })
}

/// Resolve only the written repository path. Anchor uncertainty does not make a known file absent.
pub(crate) fn local_link_target(
    root: &Path,
    source: &Path,
    destination: &str,
) -> Result<LocalTarget, LinkPathError> {
    local_link_targets(root, source, destination, None).map(|mut targets| targets.swap_remove(0))
}

/// List the paths a destination can name, in resolution order: the written
/// repository path first, then, for a site document, the page sources of its route.
pub(crate) fn local_link_targets(
    root: &Path,
    source: &Path,
    destination: &str,
    site: Option<&SiteRoutes>,
) -> Result<Vec<LocalTarget>, LinkPathError> {
    if destination.starts_with("//") || has_scheme(destination) {
        return Err(LinkPathError::External);
    }
    let raw_path = destination.split(['?', '#']).next().unwrap_or_default();
    let path = percent_decode(raw_path).ok_or(LinkPathError::Unknown)?;
    if is_template(&path) {
        return Err(LinkPathError::Template);
    }
    if path.starts_with("//") || path.starts_with("\\\\") || has_scheme(&path) {
        return Err(LinkPathError::External);
    }
    let path = path.replace('\\', "/");
    let source = if source.is_absolute() {
        source.to_path_buf()
    } else {
        root.join(source)
    };
    let physical = normalize(if path.is_empty() {
        source
    } else if path.starts_with('/') {
        root.join(path.trim_start_matches('/'))
    } else {
        source.parent().unwrap_or(root).join(&path)
    });
    let root = normalize(root);
    let relative = |candidate: &Path| {
        candidate
            .strip_prefix(&root)
            .ok()
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
    };
    let mut targets = vec![LocalTarget {
        target: relative(&physical).ok_or(LinkPathError::OutsideWorkspace)?,
        path: physical.clone(),
        route: false,
    }];
    let Some(site) = site.filter(|_| !path.is_empty()) else {
        return Ok(targets);
    };
    let rest = path.starts_with('/').then(|| site.route(&path));
    let mut routes = Vec::new();
    match rest {
        None => routes.extend(page_sources(&physical)),
        Some(Some(rest)) => {
            let page = normalize(site.root.join(rest));
            routes.push(page.clone());
            routes.extend(page_sources(&page));
        }
        Some(None) => {}
    }
    routes.retain(|route| route.starts_with(&site.root));
    if let (Some(public), Some(Some(rest))) = (&site.public, rest)
        && !rest.is_empty()
    {
        let file = normalize(public.join(rest));
        if file.starts_with(public) {
            routes.push(file);
        }
    }
    for path in routes {
        if let Some(target) = relative(&path)
            && targets.iter().all(|existing| existing.path != path)
        {
            targets.push(LocalTarget {
                path,
                target,
                route: true,
            });
        }
    }
    Ok(targets)
}

/// Markdown files a site generator renders at a route: `.html` names its
/// source page, and an extensionless route names a page or a directory index.
fn page_sources(route: &Path) -> Vec<PathBuf> {
    if route
        .extension()
        .is_some_and(|extension| extension == "html")
    {
        let mut sources = vec![route.with_extension("md")];
        // mdBook renders a directory's README.md as its index.html.
        if route.file_stem().is_some_and(|stem| stem == "index") {
            sources.push(route.with_file_name("README.md"));
        }
        return sources;
    }
    let suffixed = |suffix: &str| {
        let mut path = route.as_os_str().to_owned();
        path.push(suffix);
        PathBuf::from(path)
    };
    vec![
        suffixed(".md"),
        suffixed(".mdx"),
        route.join("index.md"),
        route.join("index.mdx"),
        route.join("README.md"),
        route.join("README.mdx"),
    ]
}

/// Choose the first candidate that exists. A directory reached only as a route
/// serves no page itself, so a later page source takes precedence over it.
pub(crate) fn select_target(
    targets: &[LocalTarget],
    mut status: impl FnMut(&LocalTarget) -> TargetStatus,
) -> (usize, TargetStatus) {
    let mut directory = None;
    for (index, target) in targets.iter().enumerate() {
        match status(target) {
            TargetStatus::Missing => {}
            TargetStatus::Directory if target.route => {
                directory.get_or_insert(index);
            }
            status => return (index, status),
        }
    }
    directory.map_or((0, TargetStatus::Missing), |index| {
        (index, TargetStatus::Directory)
    })
}

fn is_template(value: &str) -> bool {
    value.contains(['\0', '{', '}', '$', '<', '>'])
}

fn has_scheme(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            output.push(
                ((*bytes.get(index + 1)? as char).to_digit(16)? * 16
                    + (*bytes.get(index + 2)? as char).to_digit(16)?) as u8,
            );
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).ok()
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum TargetStatus {
    File,
    Directory,
    Missing,
    Unknown,
    OutsideWorkspace,
    Unreadable(String),
}

/// Inspect the nearest existing ancestor before checking a possibly absent child.
/// This keeps missing children beneath outward symlinks outside the workspace.
pub(crate) fn local_target_status(root: &Path, target: &Path) -> TargetStatus {
    let root = match root.canonicalize() {
        Ok(root) => root,
        Err(error) => {
            return TargetStatus::Unreadable(format!("Cannot inspect workspace: {error}"));
        }
    };
    let mut contained = false;
    for ancestor in target.ancestors() {
        match ancestor.canonicalize() {
            Ok(actual) => {
                if !actual.starts_with(&root) {
                    return TargetStatus::OutsideWorkspace;
                }
                contained = true;
                break;
            }
            Err(error) if missing(&error) => {}
            Err(error) => {
                return TargetStatus::Unreadable(format!("Cannot resolve link target: {error}"));
            }
        }
    }
    if !contained {
        return TargetStatus::OutsideWorkspace;
    }
    match std::fs::metadata(target) {
        Ok(metadata) if metadata.is_dir() => TargetStatus::Directory,
        Ok(_) => TargetStatus::File,
        Err(error) if missing(&error) => TargetStatus::Missing,
        Err(error) => TargetStatus::Unreadable(format!("Cannot inspect link target: {error}")),
    }
}

fn missing(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_preserves_unresolved_relative_parents() {
        assert_eq!(normalize("../../a/../b"), PathBuf::from("../../b"));
        assert_eq!(normalize("a/../../b"), PathBuf::from("../b"));
        assert_eq!(normalize("a/./b/../c"), PathBuf::from("a/c"));
    }

    #[test]
    fn site_documents_list_route_candidates_after_the_written_path() {
        let root = std::env::current_dir().unwrap();
        let site = SiteRoutes {
            root: root.join("site"),
            public: Some(root.join("site/public")),
            base: "/docs/".into(),
        };
        let targets = |destination: &str| {
            local_link_targets(
                &root,
                Path::new("site/guide/a.md"),
                destination,
                Some(&site),
            )
            .unwrap()
            .into_iter()
            .map(|target| (target.target, target.route))
            .collect::<Vec<_>>()
        };
        let route = |target: &str| (target.to_owned(), true);
        assert_eq!(
            targets("/docs/intro#x"),
            [
                ("docs/intro".to_owned(), false),
                route("site/intro"),
                route("site/intro.md"),
                route("site/intro.mdx"),
                route("site/intro/index.md"),
                route("site/intro/index.mdx"),
                route("site/intro/README.md"),
                route("site/intro/README.mdx"),
                route("site/public/intro"),
            ]
        );
        assert_eq!(
            targets("../cli/index.html"),
            [
                ("site/cli/index.html".to_owned(), false),
                route("site/cli/index.md"),
                route("site/cli/README.md"),
            ]
        );
        assert_eq!(targets("/intro"), [("intro".to_owned(), false)]);
        assert_eq!(targets("../../outside"), [("outside".to_owned(), false)]);
        assert_eq!(targets("/docs/../x"), [("x".to_owned(), false)]);
        assert_eq!(targets("#anchor"), [("site/guide/a.md".to_owned(), false)]);
    }

    #[test]
    fn route_directories_yield_to_later_page_sources() {
        let target = |target: &str, route| LocalTarget {
            path: PathBuf::from(target),
            target: target.into(),
            route,
        };
        let status = |target: &LocalTarget| match target.target.as_str() {
            "guide" | "physical" => TargetStatus::Directory,
            "guide/index.md" => TargetStatus::File,
            _ => TargetStatus::Missing,
        };
        let candidates = [
            target("missing", false),
            target("guide", true),
            target("guide.md", true),
            target("guide/index.md", true),
        ];
        assert_eq!(select_target(&candidates, status), (3, TargetStatus::File));
        assert_eq!(
            select_target(&candidates[..3], status),
            (1, TargetStatus::Directory)
        );
        assert_eq!(
            select_target(
                &[target("physical", false), target("guide/index.md", true)],
                status
            ),
            (0, TargetStatus::Directory)
        );
        assert_eq!(
            select_target(&candidates[..1], status),
            (0, TargetStatus::Missing)
        );
    }

    #[test]
    fn normalization_stops_at_the_filesystem_root() {
        let root = std::env::current_dir().unwrap();
        let filesystem_root: PathBuf = root
            .components()
            .take_while(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
            .collect();
        assert_eq!(normalize(filesystem_root.join("../../..")), filesystem_root);
    }
}

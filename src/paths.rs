//! Shared lexical paths and workspace-local link targets.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

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

#[derive(Clone, Debug, Eq, PartialEq)]
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
    // Normalization drops a trailing slash, which addresses the route's directory.
    let directory = path.ends_with('/');
    let rest = path.starts_with('/').then(|| site.route(&path));
    let mut routes = Vec::new();
    match rest {
        None => routes.extend(page_sources(&physical, directory)),
        Some(Some(rest)) => {
            let page = normalize(site.root.join(rest));
            routes.push(page.clone());
            routes.extend(page_sources(&page, directory));
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
/// A route ending in `/` names the directory index first; MkDocs and Jekyll
/// can still serve a same-named page there.
fn page_sources(route: &Path, directory: bool) -> Vec<PathBuf> {
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
    let pages = [suffixed(".md"), suffixed(".mdx")];
    let indexes = [
        route.join("index.md"),
        route.join("index.mdx"),
        route.join("README.md"),
        route.join("README.mdx"),
    ];
    if directory {
        indexes.into_iter().chain(pages).collect()
    } else {
        pages.into_iter().chain(indexes).collect()
    }
}

/// Choose the first candidate that exists. A directory reached only as a route
/// serves no page itself, so a later page source takes precedence over it.
/// A route written in lowercase reaches a page whose name has capitals, as
/// generators that lowercase routes serve it; any other letter-case mismatch
/// is the result only when no candidate exists.
pub(crate) fn select_target(
    root: &Path,
    targets: &[LocalTarget],
    mut status: impl FnMut(&LocalTarget) -> TargetStatus,
) -> (LocalTarget, TargetStatus) {
    let mut directory = None;
    let mut mismatch = None;
    for candidate in targets {
        let mut target = Cow::Borrowed(candidate);
        let mut found = status(candidate);
        if let TargetStatus::CaseMismatch(actual) = &found
            && candidate.route
            && lowercase_route(&candidate.target, actual)
        {
            target = Cow::Owned(LocalTarget {
                path: normalize(root.join(actual)),
                target: actual.clone(),
                route: true,
            });
            found = status(&target);
        }
        match found {
            TargetStatus::Missing => {}
            TargetStatus::Directory if target.route => {
                directory.get_or_insert_with(|| target.into_owned());
            }
            TargetStatus::CaseMismatch(actual) => {
                mismatch.get_or_insert_with(|| (target.into_owned(), actual));
            }
            status => return (target.into_owned(), status),
        }
    }
    if let Some(target) = directory {
        (target, TargetStatus::Directory)
    } else if let Some((target, actual)) = mismatch {
        (target, TargetStatus::CaseMismatch(actual))
    } else {
        (targets[0].clone(), TargetStatus::Missing)
    }
}

/// Each written component that differs from the entry's is its lowercase form.
fn lowercase_route(written: &str, actual: &str) -> bool {
    written
        .split('/')
        .zip(actual.split('/'))
        .all(|(written, actual)| written == actual || written == actual.to_lowercase())
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
    /// Exists under this workspace-relative spelling, which differs in letter case.
    CaseMismatch(String),
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

/// Directory entry names, read once per directory, that confirm the letter
/// case of link targets. Windows and macOS open a path under any letter case,
/// while Git and Linux treat it as a different file.
#[derive(Debug, Default)]
pub(crate) struct Listings(Mutex<HashMap<PathBuf, Option<Entries>>>);

/// Names in a directory; `None` in the cache marks one that cannot be read.
type Entries = Arc<BTreeSet<String>>;

impl Clone for Listings {
    fn clone(&self) -> Self {
        Self(Mutex::new(self.lock().clone()))
    }
}

impl Listings {
    /// Compare an inspected target with the entries on its path. A name that
    /// matches no entry exactly takes the first entry that differs only in
    /// letter case; other names stay as written for the filesystem to judge.
    pub(crate) fn confirm(&self, root: &Path, target: &Path, status: TargetStatus) -> TargetStatus {
        if !matches!(
            status,
            TargetStatus::File | TargetStatus::Directory | TargetStatus::Missing
        ) {
            return status;
        }
        let mut directory = normalize(root);
        let Ok(relative) = target.strip_prefix(&directory) else {
            return status;
        };
        let mut spelling = Vec::new();
        let mut differs = false;
        for component in relative.components() {
            let Some(name) = component.as_os_str().to_str() else {
                return status;
            };
            let entry = match self.entries(&directory) {
                Some(entries) if !entries.contains(name) => {
                    let lowercase = name.to_lowercase();
                    match entries
                        .iter()
                        .find(|entry| entry.to_lowercase() == lowercase)
                    {
                        Some(entry) => {
                            differs = true;
                            entry.clone()
                        }
                        None if status == TargetStatus::Missing => return status,
                        // Such as another Unicode normalization form on macOS.
                        None => name.to_owned(),
                    }
                }
                // An unreadable directory cannot contradict the filesystem.
                _ => name.to_owned(),
            };
            directory.push(&entry);
            spelling.push(entry);
        }
        if differs {
            TargetStatus::CaseMismatch(spelling.join("/"))
        } else {
            status
        }
    }

    fn entries(&self, directory: &Path) -> Option<Entries> {
        if let Some(entries) = self.lock().get(directory) {
            return entries.clone();
        }
        let entries = std::fs::read_dir(directory)
            .and_then(|entries| {
                entries
                    .map(|entry| entry.map(|entry| entry.file_name().into_string().ok()))
                    .collect::<Result<Vec<_>, _>>()
            })
            .ok()
            .map(|names| Arc::new(names.into_iter().flatten().collect()));
        self.lock().insert(directory.to_owned(), entries.clone());
        entries
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, Option<Entries>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
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
            targets("/docs/intro/#x"),
            [
                ("docs/intro".to_owned(), false),
                route("site/intro"),
                route("site/intro/index.md"),
                route("site/intro/index.mdx"),
                route("site/intro/README.md"),
                route("site/intro/README.mdx"),
                route("site/intro.md"),
                route("site/intro.mdx"),
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
        let select = |candidates: &[LocalTarget]| {
            let (target, status) = select_target(Path::new(""), candidates, status);
            (target.target, status)
        };
        let candidates = [
            target("missing", false),
            target("guide", true),
            target("guide.md", true),
            target("guide/index.md", true),
        ];
        assert_eq!(
            select(&candidates),
            ("guide/index.md".into(), TargetStatus::File)
        );
        assert_eq!(
            select(&candidates[..3]),
            ("guide".into(), TargetStatus::Directory)
        );
        assert_eq!(
            select(&[target("physical", false), target("guide/index.md", true)]),
            ("physical".into(), TargetStatus::Directory)
        );
        assert_eq!(
            select(&candidates[..1]),
            ("missing".into(), TargetStatus::Missing)
        );
    }

    #[test]
    fn only_lowercase_routes_reach_entries_with_capitals() {
        let target = |target: &str, route| LocalTarget {
            path: PathBuf::from(target),
            target: target.into(),
            route,
        };
        let status = |target: &LocalTarget| match target.target.as_str() {
            "Guide.md" => TargetStatus::CaseMismatch("guide.md".into()),
            "site/contributing.md" => TargetStatus::CaseMismatch("site/CONTRIBUTING.md".into()),
            "site/Contributing.md" => TargetStatus::CaseMismatch("site/contributing.md".into()),
            "site/intro" => TargetStatus::CaseMismatch("site/Intro".into()),
            "site/intro/index.md" => TargetStatus::CaseMismatch("site/Intro/index.md".into()),
            "site/guide.md" | "site/CONTRIBUTING.md" | "site/Intro/index.md" => TargetStatus::File,
            "site/Intro" => TargetStatus::Directory,
            _ => TargetStatus::Missing,
        };
        let select = |candidates: &[LocalTarget]| {
            let (target, status) = select_target(Path::new(""), candidates, status);
            (target.target, status)
        };
        let written = target("Guide.md", false);
        assert_eq!(
            select(std::slice::from_ref(&written)),
            (
                "Guide.md".into(),
                TargetStatus::CaseMismatch("guide.md".into())
            )
        );
        assert_eq!(
            select(&[written.clone(), target("site/guide.md", true)]),
            ("site/guide.md".into(), TargetStatus::File)
        );
        assert_eq!(
            select(&[target("site/contributing.md", true)]),
            ("site/CONTRIBUTING.md".into(), TargetStatus::File)
        );
        assert_eq!(
            select(&[target("site/Contributing.md", true)]),
            (
                "site/Contributing.md".into(),
                TargetStatus::CaseMismatch("site/contributing.md".into())
            )
        );
        assert_eq!(
            select(&[written, target("site/intro", true)]),
            ("site/Intro".into(), TargetStatus::Directory)
        );
        assert_eq!(
            select(&[
                target("site/intro", true),
                target("site/intro/index.md", true)
            ]),
            ("site/Intro/index.md".into(), TargetStatus::File)
        );
    }

    #[test]
    fn listings_confirm_the_letter_case_of_each_component() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("docs/Guide")).unwrap();
        std::fs::write(root.path().join("docs/Guide/setup.md"), "").unwrap();
        let listings = Listings::default();
        let confirm = |target: &str| {
            let path = normalize(root.path().join(target));
            listings.confirm(root.path(), &path, local_target_status(root.path(), &path))
        };
        assert_eq!(confirm("docs/Guide/setup.md"), TargetStatus::File);
        assert_eq!(confirm("docs/Guide"), TargetStatus::Directory);
        assert_eq!(
            confirm("DOCS/guide/setup.md"),
            TargetStatus::CaseMismatch("docs/Guide/setup.md".into())
        );
        assert_eq!(
            confirm("docs/guide"),
            TargetStatus::CaseMismatch("docs/Guide".into())
        );
        assert_eq!(confirm("docs/Guide/missing.md"), TargetStatus::Missing);
        assert_eq!(confirm("docs/Guide/setup.md/child"), TargetStatus::Missing);
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

//! Current workspace facts. Resolved paths and effective policy never enter the parse cache.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::config::Config;
use crate::diagnostics::Span;
use crate::md::{BlockKind, Document, FragmentKind};
use crate::paths::{
    LinkPathError, Listings, TargetStatus, local_link, local_link_scoped, local_target_status,
    select_target,
};
use crate::rules::{PathStatus, WorkspaceFiles};
use regex::Regex;
use serde::Serialize;

#[derive(Clone, Debug)]
pub struct IndexedFile {
    pub filename: String,
    pub path: PathBuf,
    pub document: Arc<Document>,
    pub kind: Option<String>,
    pub domain: String,
    pub enabled_rules: Vec<String>,
    pub config: Config,
}

#[derive(Clone, Debug)]
pub struct WorkspaceIndex {
    /// Containment boundary and base of stable internal file identifiers.
    pub root: PathBuf,
    /// Base of leading-`/` links written by files in the selected subtree.
    selection_root: PathBuf,
    /// Scope granted before unrelated selected-file frames widened the index.
    invocation_root: PathBuf,
    files: Vec<IndexedFile>,
    pub complete: bool,
    anchors: BTreeMap<String, OnceLock<AnchorIndex>>,
    inventory: Option<BTreeMap<String, InventoryEntryKind>>,
    /// Inventory paths by their lowercase form, built on the first missing target.
    lowercase_inventory: OnceLock<BTreeMap<String, String>>,
    listings: Listings,
}

#[derive(Clone, Debug)]
struct AnchorIndex {
    names: BTreeSet<String>,
    spans: BTreeMap<String, Span>,
}

/// A complete, immutable file listing for evaluations without materializing upstream files.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryEntryKind {
    File,
    Directory,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    External,
    Template,
    OutsideWorkspace,
    Missing,
    Directory,
    File,
    AnchorFound,
    AnchorMissing,
    AnchorUnknown,
    Unreadable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LinkResolution {
    pub target: Option<String>,
    pub anchor: Option<String>,
    pub status: LinkStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl WorkspaceIndex {
    /// Sort workspace files and defer anchor extraction until a lookup needs it.
    pub fn new(root: PathBuf, files: Vec<IndexedFile>, complete: bool) -> Self {
        Self::new_scoped(root.clone(), root, files, complete)
    }

    /// Keep the project's safety/index scope distinct from selected files'
    /// written leading-`/` link base. File names are project-relative.
    pub fn new_scoped(
        root: PathBuf,
        selection_root: PathBuf,
        files: Vec<IndexedFile>,
        complete: bool,
    ) -> Self {
        Self::new_with_scope(
            root,
            selection_root.clone(),
            selection_root,
            files,
            complete,
        )
    }

    /// Construct an index whose global lookup root can be wider than the
    /// original invocation grant. Each source may retain or expand that grant
    /// through its own effective configuration, never through another file's.
    pub fn new_with_scope(
        root: PathBuf,
        selection_root: PathBuf,
        invocation_root: PathBuf,
        mut files: Vec<IndexedFile>,
        complete: bool,
    ) -> Self {
        debug_assert!(selection_root.starts_with(&root));
        debug_assert!(invocation_root.starts_with(&root));
        files.sort_by(|left, right| left.filename.cmp(&right.filename));
        let anchors = files
            .iter()
            .map(|file| (file.filename.clone(), OnceLock::new()))
            .collect();
        Self {
            root,
            selection_root,
            invocation_root,
            files,
            complete,
            anchors,
            inventory: None,
            lowercase_inventory: OnceLock::new(),
            listings: Listings::default(),
        }
    }

    pub fn with_inventory(mut self, inventory: BTreeMap<String, InventoryEntryKind>) -> Self {
        self.inventory = Some(inventory);
        self
    }

    /// Borrow files in filename order without invalidating lookups or cached anchors.
    /// To change the file set or its documents, construct a new index.
    pub fn files(&self) -> &[IndexedFile] {
        &self.files
    }

    /// The pre-widening project boundary used to authorize each source.
    pub(crate) fn invocation_project_root(&self) -> &Path {
        &self.invocation_root
    }

    /// Consume the index to recover its files without cloning their documents.
    /// Cached anchors are discarded, so changed files require a new index.
    pub fn into_files(self) -> Vec<IndexedFile> {
        self.files
    }

    /// Look up an indexed file by its workspace-relative filename.
    pub fn file(&self, filename: &str) -> Option<&IndexedFile> {
        self.files
            .binary_search_by(|file| file.filename.as_str().cmp(filename))
            .ok()
            .map(|index| &self.files[index])
    }

    /// Return the cached anchor names, initializing this document on first use.
    pub fn anchors(&self, filename: &str) -> Option<&BTreeSet<String>> {
        Some(&self.anchor_index(filename)?.names)
    }

    /// Locate an anchor in the original source, initializing its document on demand.
    pub fn anchor_span(&self, filename: &str, anchor: &str) -> Option<Span> {
        self.anchor_index(filename)?.spans.get(anchor).copied()
    }

    /// Build anchors only for queried documents; ordinary file checks need none.
    fn anchor_index(&self, filename: &str) -> Option<&AnchorIndex> {
        let anchors = self.anchors.get(filename)?;
        Some(anchors.get_or_init(|| {
            let spans = document_anchors(&self.file(filename).unwrap().document);
            let names = spans.keys().cloned().collect();
            AnchorIndex { names, spans }
        }))
    }

    /// Resolve local links using the frozen inventory when present, otherwise the filesystem.
    /// Indexed stdin overlays exist even without a disk file.
    pub fn resolve_link(&self, source: &str, destination: &str) -> LinkResolution {
        let mut result = LinkResolution {
            target: None,
            anchor: None,
            status: LinkStatus::AnchorUnknown,
            error: None,
        };
        let site = self
            .file(source)
            .and_then(|file| file.config.site_routes(&file.path));
        let allowed_root = self
            .file(source)
            .map(|file| {
                file.config
                    .project_root_for_source_within(&self.invocation_root)
            })
            .unwrap_or_else(|| self.invocation_root.clone());
        let link = match self.local_link(source, destination, site.as_ref()) {
            Ok(link) => link,
            Err(error) => {
                result.status = match error {
                    LinkPathError::External => LinkStatus::External,
                    LinkPathError::Template => LinkStatus::Template,
                    LinkPathError::OutsideWorkspace => LinkStatus::OutsideWorkspace,
                    LinkPathError::Unknown => LinkStatus::AnchorUnknown,
                };
                return result;
            }
        };
        let (location, status) = select_target(&self.root, &link.locations, |location| {
            self.target_status(&allowed_root, &location.path, &location.target)
        });
        let mut target = location.target.clone();
        result.target = Some(target.clone());
        result.anchor = link.anchor;
        // Resolve symbolic-link aliases through filesystem identity. The target's
        // spelling is already confirmed, so this cannot merge letter-case variants.
        if self.inventory.is_none()
            && matches!(status, TargetStatus::File)
            && self.file(&target).is_none()
            && let (Ok(actual), Ok(root)) = (location.path.canonicalize(), self.root.canonicalize())
            && let Ok(relative) = actual.strip_prefix(root)
        {
            let canonical = relative.to_string_lossy().replace('\\', "/");
            if self.file(&canonical).is_some() {
                target = canonical;
                result.target = Some(target.clone());
            }
        }
        result.status = match status {
            TargetStatus::File if self.file(&target).is_some() => {
                self.indexed_status(&target, result.anchor.as_deref())
            }
            TargetStatus::File if result.anchor.as_ref().is_none_or(String::is_empty) => {
                LinkStatus::File
            }
            TargetStatus::File | TargetStatus::Unknown => LinkStatus::AnchorUnknown,
            TargetStatus::Directory => LinkStatus::Directory,
            // LNK001 reports the spelling.
            TargetStatus::CaseMismatch(_) | TargetStatus::Missing => LinkStatus::Missing,
            TargetStatus::OutsideWorkspace => LinkStatus::OutsideWorkspace,
            TargetStatus::Unreadable(error) => {
                result.error = Some(error);
                LinkStatus::Unreadable
            }
        };
        result
    }

    /// Avoid an extra root classification in the overwhelmingly common
    /// same-root case; only selected-subtree files use a child written base.
    fn local_link(
        &self,
        source: &str,
        destination: &str,
        site: Option<&crate::paths::SiteRoutes>,
    ) -> Result<crate::paths::LocalLink, LinkPathError> {
        let source_path = self
            .file(source)
            .map(|file| file.path.as_path())
            .unwrap_or_else(|| Path::new(source));
        let written_root = self.written_root(&self.root, source_path);
        if written_root.as_ref() == self.root {
            return local_link(&self.root, Path::new(source), destination, site);
        }
        local_link_scoped(
            &self.root,
            written_root.as_ref(),
            Path::new(source),
            destination,
            site,
        )
    }

    fn target_status(&self, allowed_root: &Path, path: &Path, target: &str) -> TargetStatus {
        if !path.starts_with(allowed_root) {
            return TargetStatus::OutsideWorkspace;
        }
        if let Some(inventory) = &self.inventory {
            let mut ancestor = Some(target);
            while let Some(path) = ancestor {
                if inventory.get(path) == Some(&InventoryEntryKind::Unknown) {
                    return TargetStatus::Unknown;
                }
                ancestor = path.rsplit_once('/').map(|(parent, _)| parent);
            }
            if self.file(target).is_some() {
                return TargetStatus::File;
            }
            return match inventory.get(target) {
                Some(InventoryEntryKind::Directory) => TargetStatus::Directory,
                Some(InventoryEntryKind::File) => TargetStatus::File,
                Some(InventoryEntryKind::Unknown) => TargetStatus::Unknown,
                None if target.is_empty() => TargetStatus::Directory,
                None => self
                    .lowercase_inventory
                    .get_or_init(|| {
                        let mut paths = BTreeMap::new();
                        for path in inventory.keys() {
                            paths
                                .entry(path.to_lowercase())
                                .or_insert_with(|| path.clone());
                        }
                        paths
                    })
                    .get(&target.to_lowercase())
                    .map_or(TargetStatus::Missing, |actual| {
                        TargetStatus::CaseMismatch(actual.clone())
                    }),
            };
        }
        match local_target_status(allowed_root, path) {
            status @ (TargetStatus::OutsideWorkspace | TargetStatus::Unreadable(_)) => status,
            _ if self.file(target).is_some() => TargetStatus::File,
            status => self.listings.confirm(&self.root, path, status),
        }
    }

    /// File-only destinations do not need the target's anchor index.
    fn indexed_status(&self, target: &str, anchor: Option<&str>) -> LinkStatus {
        match anchor {
            None | Some("") => LinkStatus::File,
            Some(anchor)
                if self
                    .anchors(target)
                    .is_some_and(|names| names.contains(anchor)) =>
            {
                LinkStatus::AnchorFound
            }
            Some(_) => LinkStatus::AnchorMissing,
        }
    }

    /// Stable debug output contains workspace-relative names, never runtime absolute paths.
    pub fn dump(&self) -> serde_json::Value {
        let files: Vec<_> = self.files.iter().map(|file| {
            let links: Vec<_> = file.document.links.iter().map(|link| serde_json::json!({
                "raw": link.destination,
                "span": link.span,
                "resolution": self.resolve_link(&file.filename, &link.destination),
            })).collect();
            let mut record = serde_json::json!({
                "filename": file.filename,
                "kind": file.kind,
                "domain": file.domain,
                "language": file.document.language,
                "canonical": file.document.frontmatter.as_ref().filter(|value| value.errors.is_empty()).and_then(|value| value.canonical).unwrap_or(false),
                "anchors": self.anchor_index(&file.filename).unwrap().spans,
                "identifiers": file.document.identifiers,
                "links": links,
            });
            if let Some(site) = file.config.site_for(&file.path) {
                record["site"] = serde_json::json!(site);
            }
            record
        }).collect();
        serde_json::json!({ "complete": self.complete, "files": files })
    }
}

/// Unicode category filtering follows github-slugger's documented generation rules.
pub fn github_slug(heading: &str) -> String {
    static STRIP: OnceLock<Regex> = OnceLock::new();
    let strip = STRIP.get_or_init(|| {
        Regex::new(
            r"[[\p{No}\p{Pe}\p{Pf}\p{Pi}\p{Ps}\p{Po}\p{Pd}\p{S}\p{C}\p{Z}]--[\p{Alphabetic} \-]]",
        )
        .expect("valid slug category expression")
    });
    strip
        .replace_all(&heading.to_lowercase(), "")
        .replace(' ', "-")
}

/// A heading's source ends with a literal attribute list: not inside a code
/// span, and not opened by an escaped brace, which attribute-list syntax keeps
/// as text.
fn source_ends_with_attribute_list(source: &str) -> bool {
    static ATX_CLOSING: OnceLock<Regex> = OnceLock::new();
    static SETEXT_UNDERLINE: OnceLock<Regex> = OnceLock::new();
    static LIST: OnceLock<Regex> = OnceLock::new();
    let mut lines: Vec<&str> = source
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect();
    let underline = SETEXT_UNDERLINE.get_or_init(|| Regex::new(r"^[ \t]*(?:=+|-+)$").unwrap());
    if lines.len() > 1 && lines.last().is_some_and(|line| underline.is_match(line)) {
        lines.pop();
    }
    let Some(line) = lines.last() else {
        return false;
    };
    let line = ATX_CLOSING
        .get_or_init(|| Regex::new(r"[ \t]+#+$").unwrap())
        .replace(line, "");
    LIST.get_or_init(|| Regex::new(r"(?:^|[^\\])\{:?[^{}]*\}$").unwrap())
        .is_match(&line)
}

/// Extract heading slugs and explicit HTML anchors with their original spans.
fn document_anchors(document: &Document) -> BTreeMap<String, Span> {
    let mut anchors = BTreeMap::new();
    let mut seen = BTreeSet::new();
    // Site generators read a trailing attribute list such as `{#id}` or
    // `{: #id .class}` as the heading's id and omit it from the visible text.
    static HEADING_ATTRIBUTES: OnceLock<Regex> = OnceLock::new();
    let heading_attributes =
        HEADING_ATTRIBUTES.get_or_init(|| Regex::new(r"\s*\{:?\s*([^{}]*)\}\s*$").unwrap());
    for block in document
        .blocks
        .iter()
        .filter(|block| block.kind == BlockKind::Heading)
    {
        let text = document.sections[block.section]
            .heading
            .as_deref()
            .unwrap_or_default();
        let base = github_slug(text);
        let mut slug = base.clone();
        let mut suffix = 0;
        while !seen.insert(slug.clone()) {
            suffix += 1;
            slug = format!("{base}-{suffix}");
        }
        anchors.entry(slug).or_insert(block.span);
        // The flattened text has lost code delimiters and escapes, so confirm
        // the list in the heading's source before trusting it.
        if let Some(attributes) = heading_attributes.captures(text)
            && source_ends_with_attribute_list(&document.source[block.span.start..block.span.end])
        {
            let visible = &text[..attributes.get(0).unwrap().start()];
            anchors.entry(github_slug(visible)).or_insert(block.span);
            for id in attributes[1]
                .split_whitespace()
                .filter_map(|token| token.strip_prefix('#'))
                .filter(|id| !id.is_empty())
            {
                anchors.entry(id.to_owned()).or_insert(block.span);
            }
        }
    }
    static TAG: OnceLock<Regex> = OnceLock::new();
    static ATTRIBUTE: OnceLock<Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| {
        Regex::new(r#"(?is)<([a-z][a-z0-9:-]*)\b(?:[^<>"']|"[^"]*"|'[^']*')*>"#).unwrap()
    });
    let attribute = ATTRIBUTE.get_or_init(|| {
        Regex::new(r#"(?is)\s+([^\s"'=<>`/]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?"#)
            .unwrap()
    });
    let mut ignored: Vec<Span> = document
        .blocks
        .iter()
        .filter(|block| matches!(block.kind, BlockKind::Code | BlockKind::HtmlComment))
        .map(|block| block.span)
        .collect();
    ignored.extend(document.comments.iter().map(|comment| comment.span));
    ignored.extend(document.frontmatter.iter().map(|value| value.span));
    ignored.extend(
        document
            .sentences
            .iter()
            .flat_map(|sentence| &sentence.fragments)
            .filter(|fragment| {
                matches!(
                    fragment.kind,
                    FragmentKind::InlineCode | FragmentKind::LinkDestination
                )
            })
            .map(|fragment| fragment.span),
    );
    let mut raw_text_end = 0;
    for capture in tag.captures_iter(&document.source) {
        let matched = capture.get(0).unwrap();
        let inside_html = document.blocks.iter().any(|block| {
            block.kind == BlockKind::Html
                && block.span.start <= matched.start()
                && matched.start() < block.span.end
        });
        if matched.start() < raw_text_end
            || ignored
                .iter()
                .any(|span| span.start <= matched.start() && matched.start() < span.end)
            || (!inside_html
                && document.source[..matched.start()]
                    .bytes()
                    .rev()
                    .take_while(|&byte| byte == b'\\')
                    .count()
                    % 2
                    != 0)
        {
            continue;
        }
        let element = capture[1].to_ascii_lowercase();
        if matches!(
            element.as_str(),
            "script"
                | "style"
                | "textarea"
                | "title"
                | "xmp"
                | "iframe"
                | "noembed"
                | "noframes"
                | "plaintext"
        ) {
            raw_text_end = document.source[matched.end()..]
                .to_ascii_lowercase()
                .find(&format!("</{element}"))
                .map_or(document.source.len(), |offset| matched.end() + offset);
        }
        for attr in attribute.captures_iter(matched.as_str()) {
            if !(attr[1].eq_ignore_ascii_case("id")
                || attr[1].eq_ignore_ascii_case("name") && capture[1].eq_ignore_ascii_case("a"))
            {
                continue;
            }
            let Some(value) = attr.get(2).or_else(|| attr.get(3)).or_else(|| attr.get(4)) else {
                continue;
            };
            anchors
                .entry(decode_html(value.as_str()))
                .or_insert(Span::new(matched.start(), matched.end()));
        }
    }
    anchors
}

fn decode_html(value: &str) -> String {
    static ENTITY: OnceLock<Regex> = OnceLock::new();
    ENTITY
        .get_or_init(|| {
            Regex::new(r"&(#(?:x|X)[0-9a-fA-F]+|#[0-9]+|[a-zA-Z][a-zA-Z0-9]+);").unwrap()
        })
        .replace_all(value, |capture: &regex::Captures<'_>| {
            let entity = &capture[1];
            if let Some(number) = entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
            {
                decode_numeric(number, 16)
            } else if let Some(number) = entity.strip_prefix('#') {
                decode_numeric(number, 10)
            } else {
                markdown::decode_named(entity, true).unwrap_or_else(|| capture[0].to_owned())
            }
        })
        .into_owned()
}

fn decode_numeric(number: &str, radix: u32) -> String {
    let codepoint = u32::from_str_radix(number, radix).unwrap_or(0xFFFD);
    // HTML's numeric-reference replacement table retains its Windows-1252 aliases.
    let codepoint = match codepoint {
        0 => 0xFFFD,
        0x80 => 0x20AC,
        0x82 => 0x201A,
        0x83 => 0x0192,
        0x84 => 0x201E,
        0x85 => 0x2026,
        0x86 => 0x2020,
        0x87 => 0x2021,
        0x88 => 0x02C6,
        0x89 => 0x2030,
        0x8A => 0x0160,
        0x8B => 0x2039,
        0x8C => 0x0152,
        0x8E => 0x017D,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0x93 => 0x201C,
        0x94 => 0x201D,
        0x95 => 0x2022,
        0x96 => 0x2013,
        0x97 => 0x2014,
        0x98 => 0x02DC,
        0x99 => 0x2122,
        0x9A => 0x0161,
        0x9B => 0x203A,
        0x9C => 0x0153,
        0x9E => 0x017E,
        0x9F => 0x0178,
        value => value,
    };
    char::from_u32(codepoint)
        .unwrap_or(char::REPLACEMENT_CHARACTER)
        .to_string()
}

impl WorkspaceFiles for WorkspaceIndex {
    fn invocation_root<'a>(&'a self, _workspace_root: &'a Path) -> &'a Path {
        self.invocation_project_root()
    }

    fn written_root<'a>(&'a self, _workspace_root: &'a Path, source_path: &Path) -> Cow<'a, Path> {
        let selected = if source_path.is_absolute() {
            source_path.starts_with(&self.selection_root)
        } else {
            self.root
                .join(source_path)
                .starts_with(&self.selection_root)
        };
        if selected {
            Cow::Borrowed(self.selection_root.as_path())
        } else {
            source_path
                .strip_prefix(&self.root)
                .ok()
                .and_then(|relative| self.file(&relative.to_string_lossy().replace('\\', "/")))
                .map(|file| {
                    Cow::Owned(
                        file.config
                            .project_root_for_source_within(&self.invocation_root),
                    )
                })
                .unwrap_or_else(|| Cow::Borrowed(self.invocation_root.as_path()))
        }
    }

    fn status(&self, _workspace_root: &Path, target: &Path) -> PathStatus {
        let Ok(relative) = target.strip_prefix(&self.root) else {
            return PathStatus::Unknown;
        };
        self.target_status(
            _workspace_root,
            target,
            &relative.to_string_lossy().replace('\\', "/"),
        )
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(root: &Path, filename: &str, source: &str) -> IndexedFile {
        IndexedFile {
            filename: filename.into(),
            path: root.join(filename),
            document: crate::md::parse(source).unwrap().into(),
            kind: Some("reference".into()),
            domain: String::new(),
            enabled_rules: Vec::new(),
            config: Config::defaults(root).unwrap(),
        }
    }

    /// Owned files can be changed only after consuming and rebuilding the index.
    #[test]
    fn rebuilding_owned_files_restores_order_and_refreshes_anchors() {
        let root = tempfile::tempdir().unwrap();
        let index = WorkspaceIndex::new(
            root.path().to_path_buf(),
            vec![
                file(root.path(), "b.md", "# Other\n"),
                file(root.path(), "a.md", "# Before\n"),
            ],
            true,
        );
        assert_eq!(index.files()[0].filename, "a.md");
        assert!(index.anchors("a.md").unwrap().contains("before"));
        let original_document = Arc::clone(&index.files()[1].document);

        let mut files = index.into_files();
        assert!(Arc::ptr_eq(&original_document, &files[1].document));
        files[0] = file(root.path(), "renamed.md", "# After\n");
        let rebuilt = WorkspaceIndex::new(root.path().to_path_buf(), files, true);
        assert_eq!(rebuilt.files()[0].filename, "b.md");
        assert!(rebuilt.file("a.md").is_none());
        assert!(rebuilt.anchors("a.md").is_none());
        assert!(rebuilt.file("renamed.md").is_some());
        let anchors = rebuilt.anchors("renamed.md").unwrap();
        assert!(anchors.contains("after"));
        assert!(!anchors.contains("before"));
    }

    /// File-only checks and a target lookup leave unrelated anchor indexes lazy.
    #[test]
    fn anchors_are_built_on_demand_and_dump_still_includes_every_file() {
        let root = tempfile::tempdir().unwrap();
        let index = WorkspaceIndex::new(
            root.path().to_path_buf(),
            vec![
                file(root.path(), "a.md", "# Source\n"),
                file(root.path(), "b.md", "# Target\n# Target\n"),
            ],
            true,
        )
        .with_inventory(BTreeMap::new());
        for destination in ["b.md", "b.md#", "missing.md#anchor"] {
            index.resolve_link("a.md", destination);
        }
        assert!(index.anchors("missing.md").is_none());
        assert!(index.anchor_span("missing.md", "anchor").is_none());
        assert!(
            index
                .anchors
                .values()
                .all(|anchors| anchors.get().is_none())
        );

        assert_eq!(
            index.resolve_link("a.md", "b.md#target-1").status,
            LinkStatus::AnchorFound
        );
        assert!(index.anchors["a.md"].get().is_none());
        let first = index.anchors("b.md").unwrap();
        assert!(std::ptr::eq(first, index.anchors("b.md").unwrap()));
        assert_eq!(
            index.anchor_span("b.md", "target-1"),
            Some(Span::new(9, 17))
        );
        assert_eq!(
            index.resolve_link("a.md", "b.md#absent").status,
            LinkStatus::AnchorMissing
        );

        let dump = index.dump();
        assert!(dump["files"][0]["anchors"].get("source").is_some());
        assert!(dump["files"][1]["anchors"].get("target-1").is_some());
        assert!(
            index
                .anchors
                .values()
                .all(|anchors| anchors.get().is_some())
        );
        assert_eq!(dump, index.dump());
    }

    #[test]
    fn github_anchors_cover_rendered_text_cjk_unicode_and_collision_suffixes() {
        let root = tempfile::tempdir().unwrap();
        let source = "# Hello, *world*!\n# Hello world\n# Hello world-1\n# Hello world\n# 安装：配置！\n# 日本語・ガイド\n# Déjà vu 🦀\n# A <em>formatted</em> `value`\n# Hello!  World\n# First. *Next*\n";
        let index = WorkspaceIndex::new(
            root.path().to_path_buf(),
            vec![file(root.path(), "a.md", source)],
            true,
        );
        let anchors = index.anchors("a.md").unwrap();
        for expected in [
            "hello-world",
            "hello-world-1",
            "hello-world-1-1",
            "hello-world-2",
            "安装配置",
            "日本語ガイド",
            "déjà-vu-",
            "a-formatted-value",
            "hello--world",
            "first-next",
        ] {
            assert!(
                anchors.contains(expected),
                "missing {expected:?} from {anchors:?}"
            );
        }
        assert_eq!(github_slug("one  two_name — x"), "one--two_name--x");
        assert!(index.anchor_span("a.md", "安装配置").is_some());
    }

    #[test]
    fn heading_attribute_lists_add_declared_ids_and_the_visible_slug() {
        let root = tempfile::tempdir().unwrap();
        let source = "# Array some {#array-some}\n## 安装 { #install .note }\n## Options {: #opts }\n## Classes only {.wide}\n## Template {name}\n# Empty {#}\n";
        let index = WorkspaceIndex::new(
            root.path().to_path_buf(),
            vec![file(root.path(), "a.md", source)],
            true,
        );
        let anchors = index.anchors("a.md").unwrap();
        for expected in [
            "array-some",
            "array-some-array-some",
            "install",
            "安装",
            "opts",
            "options",
            "classes-only",
            "template",
            "template-name",
        ] {
            assert!(
                anchors.contains(expected),
                "missing {expected:?} from {anchors:?}"
            );
        }
        assert!(!anchors.contains("") && !anchors.contains("name"));
        assert_eq!(
            index.anchor_span("a.md", "install"),
            index.anchor_span("a.md", "安装")
        );
    }

    #[test]
    fn heading_attribute_lists_in_code_or_after_an_escape_are_text() {
        let root = tempfile::tempdir().unwrap();
        let source = "# Example `{#ghost}`\n\n# Escaped \\{#escaped}\n\n# Closed {#closed} ##\n\nSetext {#setext}\n===\n";
        let index = WorkspaceIndex::new(
            root.path().to_path_buf(),
            vec![file(root.path(), "a.md", source)],
            true,
        );
        let anchors = index.anchors("a.md").unwrap();
        for expected in ["example-ghost", "escaped-escaped", "closed", "setext"] {
            assert!(
                anchors.contains(expected),
                "missing {expected:?} from {anchors:?}"
            );
        }
        for absent in ["ghost", "escaped"] {
            assert!(
                !anchors.contains(absent),
                "unexpected {absent:?} in {anchors:?}"
            );
        }
    }

    #[test]
    fn html_ids_and_names_exclude_comments_code_frontmatter_and_escaped_tags() {
        let root = tempfile::tempdir().unwrap();
        let source = "---\nnote: '<a id=frontmatter>'\n---\n# Title\n\n<a name='old'></a> <span ID=custom></span> <a id=two&amp;three></a>\n\n<div id=block></div>\n\n`<a id=inline>`\n\n```html\n<a id=fenced>\n```\n\n<!-- <a id=comment> -->\n\n\\<a id=escaped>\n\n<span name=not-an-anchor></span>\n\n<span title='the id=quoted'></span>\n";
        let index = WorkspaceIndex::new(
            root.path().to_path_buf(),
            vec![file(root.path(), "a.md", source)],
            true,
        );
        let anchors = index.anchors("a.md").unwrap();
        for expected in ["title", "old", "custom", "two&three", "block"] {
            assert!(anchors.contains(expected), "{expected}: {anchors:?}");
        }
        for absent in [
            "frontmatter",
            "inline",
            "fenced",
            "comment",
            "escaped",
            "not-an-anchor",
            "quoted",
        ] {
            assert!(!anchors.contains(absent), "{absent}: {anchors:?}");
        }
    }

    #[test]
    fn links_decode_paths_and_anchors_and_keep_excluded_targets_unknown() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("docs")).unwrap();
        std::fs::write(root.path().join("excluded.md"), "# Known only on disk").unwrap();
        std::fs::write(root.path().join("image.png"), "image").unwrap();
        let index = WorkspaceIndex::new(
            root.path().to_path_buf(),
            vec![
                file(root.path(), "docs/from.md", "# From"),
                file(
                    root.path(),
                    "hello world.md",
                    "---\nkind: generated\n---\n# 标题\n",
                ),
            ],
            true,
        );
        for (destination, expected) in [
            (
                "../hello%20world.md#%E6%A0%87%E9%A2%98",
                LinkStatus::AnchorFound,
            ),
            ("/hello%20world.md#missing", LinkStatus::AnchorMissing),
            ("../excluded.md#whatever", LinkStatus::AnchorUnknown),
            ("../image.png#fragment", LinkStatus::AnchorUnknown),
            ("../image.png", LinkStatus::File),
            ("../docs", LinkStatus::Directory),
            ("../missing.md#missing", LinkStatus::Missing),
            ("../../../external.md#x", LinkStatus::OutsideWorkspace),
            ("../${name}.md", LinkStatus::Template),
            ("../hello%20world.md#{{anchor}}", LinkStatus::Template),
            (
                "../hello%20world.md?q=%7Bname%7D#missing",
                LinkStatus::Template,
            ),
            ("https://example.com/#x", LinkStatus::External),
            ("//example.com/#x", LinkStatus::External),
            ("#from", LinkStatus::AnchorFound),
            ("?view=1#from", LinkStatus::AnchorFound),
            ("#", LinkStatus::File),
            ("%ZZ.md", LinkStatus::AnchorUnknown),
        ] {
            assert_eq!(
                index.resolve_link("docs/from.md", destination).status,
                expected,
                "{destination}"
            );
        }
        assert_eq!(
            index
                .resolve_link("docs/from.md", "../hello%20world.md#标题")
                .target
                .as_deref(),
            Some("hello world.md")
        );
    }

    #[test]
    fn moved_source_recomputes_links_and_dump_is_order_independent() {
        let root = tempfile::tempdir().unwrap();
        let first = file(root.path(), "a.md", "[to](target.md#yes)");
        let moved = file(root.path(), "sub/a.md", &first.document.source);
        let target = file(root.path(), "target.md", "# Yes");
        let index = WorkspaceIndex::new(
            root.path().to_path_buf(),
            vec![first.clone(), moved.clone(), target.clone()],
            true,
        );
        assert_eq!(
            index.resolve_link("a.md", "target.md#yes").status,
            LinkStatus::AnchorFound
        );
        assert_eq!(
            index.resolve_link("sub/a.md", "target.md#yes").status,
            LinkStatus::Missing
        );
        let reversed =
            WorkspaceIndex::new(root.path().to_path_buf(), vec![target, moved, first], true);
        assert_eq!(index.dump(), reversed.dump());
        assert!(
            !index
                .dump()
                .to_string()
                .contains(root.path().to_str().unwrap())
        );
    }

    #[test]
    fn raw_html_text_is_not_parsed_as_nested_elements_and_entities_decode() {
        let root = tempfile::tempdir().unwrap();
        let source = "<script id=script>const example = '<a id=fake>';</script>\n\n<a id='&copy;&#xE9;&#233;'></a>\n\n<textarea><a id=also-fake></textarea>\n";
        let index = WorkspaceIndex::new(
            root.path().to_path_buf(),
            vec![file(root.path(), "a.md", source)],
            true,
        );
        let anchors = index.anchors("a.md").unwrap();
        assert!(anchors.contains("script"));
        assert!(anchors.contains("©éé"));
        assert!(!anchors.contains("fake"));
        assert!(!anchors.contains("also-fake"));
        assert_eq!(decode_html("&#128;&#x110000;&#99999999999999999;"), "€��");
    }

    #[test]
    fn inventory_matches_disk_resolution_and_never_reads_unknown_subtrees() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("docs")).unwrap();
        std::fs::write(root.path().join("asset.png"), "image").unwrap();
        std::fs::write(root.path().join("excluded.md"), "# Excluded").unwrap();
        let files = vec![
            file(root.path(), "docs/from.md", "# From"),
            file(root.path(), "target.md", "# 标题"),
        ];
        let disk = WorkspaceIndex::new(root.path().to_path_buf(), files.clone(), true);
        let inventory = WorkspaceIndex::new(root.path().to_path_buf(), files, true).with_inventory(
            BTreeMap::from([
                ("docs".into(), InventoryEntryKind::Directory),
                ("asset.png".into(), InventoryEntryKind::File),
                ("excluded.md".into(), InventoryEntryKind::File),
                ("submodule".into(), InventoryEntryKind::Unknown),
            ]),
        );
        for destination in [
            "#from",
            "../target.md#标题",
            "/target.md#absent",
            "../asset.png",
            "../asset.png#x",
            "../excluded.md#x",
            "/docs",
            "/",
            "../../outside.md",
            "../missing.md",
            "../target.md?q={{name}}",
        ] {
            assert_eq!(
                disk.resolve_link("docs/from.md", destination),
                inventory.resolve_link("docs/from.md", destination),
                "{destination}"
            );
        }
        assert_eq!(
            inventory
                .resolve_link("docs/from.md", "../submodule/missing.md#x")
                .status,
            LinkStatus::AnchorUnknown
        );
        // Virtual snapshots use their inventory even when the workspace root does not exist.
        let absent = root.path().join("absent-workspace");
        let virtual_index = WorkspaceIndex::new(absent, Vec::new(), true).with_inventory(
            BTreeMap::from([("file.md".into(), InventoryEntryKind::File)]),
        );
        assert_eq!(
            virtual_index.resolve_link("from.md", "file.md").status,
            LinkStatus::File
        );
        assert_eq!(
            virtual_index.resolve_link("from.md", "missing.md").status,
            LinkStatus::Missing
        );
    }

    #[test]
    fn inventory_matches_disk_resolution_for_site_routes() {
        let root = tempfile::tempdir().unwrap();
        for directory in ["site/guide", "site/public"] {
            std::fs::create_dir_all(root.path().join(directory)).unwrap();
        }
        std::fs::write(root.path().join("site/public/logo.png"), "image").unwrap();
        let config = Config::parse(
            "[[sites]]\npath = 'site/**'\nroot = 'site'\npublic = 'site/public'\n",
            root.path(),
        )
        .unwrap();
        let mut from = file(root.path(), "site/guide/from.md", "# From");
        from.config = config;
        let files = vec![
            from,
            file(root.path(), "site/guide/index.md", "# Guide"),
            file(root.path(), "site/guide/setup.md", "# Setup"),
        ];
        let disk = WorkspaceIndex::new(root.path().to_path_buf(), files.clone(), true);
        let inventory = WorkspaceIndex::new(root.path().to_path_buf(), files, true).with_inventory(
            BTreeMap::from([
                ("site".into(), InventoryEntryKind::Directory),
                ("site/guide".into(), InventoryEntryKind::Directory),
                ("site/public".into(), InventoryEntryKind::Directory),
                ("site/public/logo.png".into(), InventoryEntryKind::File),
            ]),
        );
        for (destination, target, status) in [
            (
                "/guide/setup#setup",
                "site/guide/setup.md",
                LinkStatus::AnchorFound,
            ),
            (
                "setup.html#absent",
                "site/guide/setup.md",
                LinkStatus::AnchorMissing,
            ),
            (
                "/guide/#guide",
                "site/guide/index.md",
                LinkStatus::AnchorFound,
            ),
            ("/logo.png", "site/public/logo.png", LinkStatus::File),
            ("/missing", "missing", LinkStatus::Missing),
        ] {
            let resolution = disk.resolve_link("site/guide/from.md", destination);
            assert_eq!(resolution.target.as_deref(), Some(target), "{destination}");
            assert_eq!(resolution.status, status, "{destination}");
            assert_eq!(
                resolution,
                inventory.resolve_link("site/guide/from.md", destination),
                "{destination}"
            );
        }
    }

    /// A promoted project admits inherited routes without changing `/` in
    /// Markdown written in the selected child directory.
    #[test]
    fn scoped_index_keeps_child_written_links_and_parent_site_routes() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp");
        std::fs::create_dir_all(&fixtures).unwrap();
        let project = tempfile::Builder::new()
            .prefix("scoped-index-")
            .tempdir_in(fixtures)
            .unwrap();
        let root = project.path();
        let child = root.join("docs");
        std::fs::create_dir_all(root.join("public")).unwrap();
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("local.md"), "# Local\n").unwrap();
        std::fs::write(root.join("sibling.md"), "# Sibling\n").unwrap();
        std::fs::write(root.join("public/logo.svg"), "logo").unwrap();
        let config = Config::parse(
            "preview = true\n[lint]\nselect = ['LNK001', 'PTR001']\n[[sites]]\npath = 'docs/**'\nroot = '.'\npublic = 'public'\n",
            root,
        )
        .unwrap();
        let mut source = file(
            root,
            "docs/a.md",
            "---\nkind: reference\n---\n# Source\n\n[Local](/local.md) [Sibling](/sibling.md) [Logo](/logo.svg)\n\nSee [configuration](/).\n",
        );
        source.config = config.clone();
        let index = WorkspaceIndex::new_scoped(
            root.to_path_buf(),
            child,
            vec![
                source,
                file(root, "docs/local.md", "# Local\n"),
                file(root, "sibling.md", "# Sibling\n"),
            ],
            true,
        );
        for (source, destination, target, status) in [
            ("docs/a.md", "/local.md", "docs/local.md", LinkStatus::File),
            ("docs/a.md", "../sibling.md", "sibling.md", LinkStatus::File),
            ("docs/a.md", "/sibling.md", "sibling.md", LinkStatus::File),
            (
                "docs/a.md",
                "/logo.svg",
                "public/logo.svg",
                LinkStatus::File,
            ),
            ("sibling.md", "/sibling.md", "sibling.md", LinkStatus::File),
        ] {
            let resolved = index.resolve_link(source, destination);
            assert_eq!(resolved.target.as_deref(), Some(target), "{destination}");
            assert_eq!(resolved.status, status, "{destination}");
        }
        let source = index.file("docs/a.md").unwrap();
        let context = crate::rules::CheckContext {
            document: &source.document,
            filename: &source.filename,
            path: &source.path,
            workspace_root: root,
            config: &config,
            overrides: &crate::config::CliOverrides::default(),
        };
        let checked = crate::rules::check_raw_with_files(&context, &index).unwrap();
        assert_eq!(
            checked
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>(),
            ["PTR001"]
        );
    }

    /// The selected child's ancestor grant is not inherited by an unrelated
    /// sibling policy merely because both files share one internal index.
    #[test]
    fn widened_index_does_not_authorize_neighbor_links_or_anchors() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp");
        std::fs::create_dir_all(&fixtures).unwrap();
        let project = tempfile::Builder::new()
            .prefix("scoped-neighbor-")
            .tempdir_in(fixtures)
            .unwrap();
        let root = project.path();
        let selected = root.join("docs");
        let neighbor = root.join("other");
        std::fs::create_dir_all(&selected).unwrap();
        std::fs::create_dir_all(&neighbor).unwrap();
        std::fs::write(root.join("secret.md"), "# Secret\n").unwrap();
        std::fs::write(neighbor.join("local.md"), "# Local\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("secret.md"), neighbor.join("alias.md")).unwrap();
        let config = |directory: &Path| {
            Config::parse("preview = true\n[lint]\nselect = ['LNK001']\n", directory).unwrap()
        };
        let mut selected_file = file(root, "docs/a.md", "[secret](../secret.md#secret)");
        selected_file.config = config(root);
        let mut neighbor_file = file(root, "other/a.md", "[secret](../secret.md#secret)");
        neighbor_file.config = config(&neighbor);
        let files = vec![
            selected_file,
            neighbor_file,
            file(root, "secret.md", "# Secret\n"),
            file(root, "other/local.md", "# Local\n"),
        ];
        let index = WorkspaceIndex::new_scoped(root.to_path_buf(), selected, files.clone(), true);
        assert_eq!(
            index
                .resolve_link("docs/a.md", "../secret.md#secret")
                .status,
            LinkStatus::AnchorFound
        );
        assert_eq!(
            index
                .resolve_link("other/a.md", "../secret.md#secret")
                .status,
            LinkStatus::OutsideWorkspace
        );
        assert_eq!(
            index.resolve_link("other/a.md", "/local.md").status,
            LinkStatus::File
        );
        #[cfg(unix)]
        assert_eq!(
            index.resolve_link("other/a.md", "alias.md#secret").status,
            LinkStatus::OutsideWorkspace
        );
        let source = index.file("other/a.md").unwrap();
        let context = crate::rules::CheckContext {
            document: &source.document,
            filename: &source.filename,
            path: &source.path,
            workspace_root: root,
            config: &source.config,
            overrides: &crate::config::CliOverrides::default(),
        };
        let checked = crate::rules::check_raw_with_files(&context, &index).unwrap();
        assert!(checked.diagnostics.is_empty());
        assert!(checked.incomplete_rules.contains("LNK001"));

        // A root invocation retains its historical grant even when a file's
        // nearest configuration is nested below that root.
        let root_index = WorkspaceIndex::new_with_scope(
            root.to_path_buf(),
            root.to_path_buf(),
            root.to_path_buf(),
            files,
            true,
        );
        assert_eq!(
            root_index
                .resolve_link("other/a.md", "../secret.md#secret")
                .status,
            LinkStatus::AnchorFound
        );
    }

    #[test]
    fn invalid_frontmatter_cannot_claim_canonical_ownership_in_dump() {
        let root = tempfile::tempdir().unwrap();
        let source = "---\nkind: [reference]\ncanonical: true\n---\n# Fields\n";
        let file = file(root.path(), "a.md", source);
        assert!(
            !file
                .document
                .frontmatter
                .as_ref()
                .unwrap()
                .errors
                .is_empty()
        );
        let index = WorkspaceIndex::new(root.path().to_path_buf(), vec![file], true);
        assert_eq!(index.dump()["files"][0]["canonical"], false);
    }

    #[cfg(unix)]
    #[test]
    fn links_below_external_symlinks_remain_unknown_even_when_target_is_missing() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("outside")).unwrap();
        let index = WorkspaceIndex::new(
            root.path().to_path_buf(),
            vec![file(root.path(), "a.md", "# A")],
            true,
        );
        assert_eq!(
            index.resolve_link("a.md", "outside/missing.md#x").status,
            LinkStatus::OutsideWorkspace
        );
    }

    #[cfg(unix)]
    #[test]
    fn promoted_project_still_rejects_symlink_targets_outside_project() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp");
        std::fs::create_dir_all(&fixtures).unwrap();
        let project = tempfile::Builder::new()
            .prefix("scoped-project-")
            .tempdir_in(&fixtures)
            .unwrap();
        let outside = tempfile::Builder::new()
            .prefix("scoped-outside-")
            .tempdir_in(fixtures)
            .unwrap();
        std::fs::create_dir(project.path().join("docs")).unwrap();
        std::os::unix::fs::symlink(outside.path(), project.path().join("public")).unwrap();
        let config = Config::parse(
            "[[sites]]\npath = 'docs/**'\nroot = '.'\npublic = 'public'\n",
            project.path(),
        )
        .unwrap();
        let mut source = file(project.path(), "docs/a.md", "# Source\n");
        source.config = config;
        let index = WorkspaceIndex::new_scoped(
            project.path().to_path_buf(),
            project.path().join("docs"),
            vec![source],
            true,
        );
        assert_eq!(
            index.resolve_link("docs/a.md", "/logo.svg").status,
            LinkStatus::OutsideWorkspace
        );
    }
}

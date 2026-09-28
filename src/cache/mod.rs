//! Best-effort content cache. Filesystem paths, policy and diagnostics are never cached.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once, Weak};
use std::time::{Duration, SystemTime};

use crate::diagnostics::Span;
use crate::md::{Document, ParseError};
use bincode::Options;
use sha2::{Digest, Sha256};

const SCHEMA: &str = "seiso-parse-cache-5-bincode-varint-le";
const HEADER: &[u8] = b"seiso-parse-cache-5-bincode-varint-le\n";
const GITIGNORE: &str = "# Automatically created by seiso.\n*\n";
const CACHEDIR_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n# This file is a cache directory tag created by seiso.\n# For information about cache directory tags see https://bford.info/cachedir/\n";
const PRUNE_MARKER: &str = ".pruned";
const PRUNE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_ENTRY_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Clone, Debug)]
pub struct ParseCache {
    directory: PathBuf,
    enabled: bool,
    documents: Arc<Mutex<HashMap<[u8; 32], Weak<Document>>>>,
    maintained: Arc<Once>,
}

impl ParseCache {
    pub fn new(directory: PathBuf, enabled: bool) -> Self {
        Self {
            directory,
            enabled,
            documents: Arc::default(),
            maintained: Arc::new(Once::new()),
        }
    }

    pub fn parse(&self, source: &str) -> Result<Document, ParseError> {
        self.parse_shared(source).map(Arc::unwrap_or_clone)
    }

    /// Reuse immutable content models already held by this check, without retaining them afterward.
    pub fn parse_shared(&self, source: &str) -> Result<Arc<Document>, ParseError> {
        if !self.enabled {
            return crate::md::parse(source).map(Arc::new);
        }
        let key = self.key(source);
        if let Ok(documents) = self.documents.lock()
            && let Some(document) = documents.get(&key).and_then(Weak::upgrade)
        {
            return Ok(document);
        }
        let path = self.path_for_key(&key);
        let document = Arc::new(match self.read(&path, source) {
            Some(document) => document,
            None => {
                let document = crate::md::parse(source)?;
                self.write(&path, &document);
                document
            }
        });
        if let Ok(mut documents) = self.documents.lock() {
            documents.insert(key, Arc::downgrade(&document));
        }
        Ok(document)
    }

    #[cfg(test)]
    fn entry_path(&self, source: &str) -> PathBuf {
        self.path_for_key(&self.key(source))
    }

    fn key(&self, source: &str) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(SCHEMA);
        hash.update([0]);
        hash.update(env!("CARGO_PKG_VERSION"));
        hash.update([0]);
        hash.update(source.as_bytes());
        hash.finalize().into()
    }

    fn path_for_key(&self, key: &[u8; 32]) -> PathBuf {
        let key: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
        self.directory.join(format!("{key}.cache"))
    }

    fn read(&self, path: &std::path::Path, source: &str) -> Option<Document> {
        let bytes = std::fs::read(path).ok()?;
        let bytes = bytes.strip_prefix(HEADER)?;
        let (checksum, payload) = bytes.split_at_checked(32)?;
        if Sha256::digest(payload).as_slice() != checksum {
            return None;
        }
        let document: Document = bincode::options()
            .with_little_endian()
            .with_varint_encoding()
            .with_limit(payload.len() as u64)
            .reject_trailing_bytes()
            .deserialize(payload)
            .ok()?;
        if document.source != source || !valid(&document) {
            return None;
        }
        Some(document)
    }

    fn write(&self, path: &std::path::Path, document: &Document) {
        let Ok(payload) = bincode::options()
            .with_little_endian()
            .with_varint_encoding()
            .reject_trailing_bytes()
            .serialize(document)
        else {
            return;
        };
        if std::fs::create_dir_all(&self.directory).is_err() {
            return;
        }
        self.maintained
            .call_once(|| self.maintain(SystemTime::now()));
        let Ok(mut temporary) = tempfile::NamedTempFile::new_in(&self.directory) else {
            return;
        };
        let result = temporary
            .write_all(HEADER)
            .and_then(|()| temporary.write_all(&Sha256::digest(&payload)))
            .and_then(|()| temporary.write_all(&payload))
            .and_then(|()| temporary.flush());
        if result.is_ok() {
            let _ = temporary.persist(path);
        }
    }

    /// Keep the directory out of version control and backups, and bound its growth.
    ///
    /// Entries are content-addressed, so edited sources and older seiso versions
    /// leave entries that are never read again. Removing any entry only costs a
    /// later parse.
    fn maintain(&self, now: SystemTime) {
        for (name, contents) in [(".gitignore", GITIGNORE), ("CACHEDIR.TAG", CACHEDIR_TAG)] {
            let path = self.directory.join(name);
            if !path.exists() {
                let _ = std::fs::write(path, contents);
            }
        }
        let marker = self.directory.join(PRUNE_MARKER);
        let pruned_recently = std::fs::metadata(&marker)
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|time| {
                now.duration_since(time)
                    .is_ok_and(|age| age < PRUNE_INTERVAL)
            });
        if pruned_recently || std::fs::write(&marker, b"").is_err() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !(name.ends_with(".cache") || name.starts_with(".tmp")) {
                continue;
            }
            let expired = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .is_ok_and(|time| {
                    now.duration_since(time)
                        .is_ok_and(|age| age > MAX_ENTRY_AGE)
                });
            if expired {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Validate arena references and UTF-8 spans before consumers can index cached data.
fn valid(document: &Document) -> bool {
    let source = &document.source;
    let span = |span: Span| span.start <= span.end && source.get(span.start..span.end).is_some();
    let Some(root) = document.sections.first() else {
        return false;
    };
    if root.parent.is_some() || root.span != Span::new(0, source.len()) {
        return false;
    }
    if document.frontmatter.as_ref().is_some_and(|value| {
        !span(value.span) || value.errors.iter().any(|error| !span(error.span))
    }) {
        return false;
    }
    let mut section_children = vec![0_usize; document.sections.len()];
    let mut section_blocks = vec![0_usize; document.blocks.len()];
    let mut block_children = vec![0_usize; document.blocks.len()];
    let mut block_sentences = vec![0_usize; document.sentences.len()];
    for (index, section) in document.sections.iter().enumerate() {
        if !span(section.span)
            || section.heading_span.is_some_and(|value| !span(value))
            || (index > 0 && section.parent.is_none_or(|parent| parent >= index))
        {
            return false;
        }
        for &child in &section.children {
            if document
                .sections
                .get(child)
                .is_none_or(|value| value.parent != Some(index))
            {
                return false;
            }
            section_children[child] += 1;
        }
        for &block in &section.blocks {
            if document
                .blocks
                .get(block)
                .is_none_or(|value| value.section != index)
            {
                return false;
            }
            section_blocks[block] += 1;
        }
    }
    if section_children[0] != 0
        || section_children[1..].iter().any(|&count| count != 1)
        || section_blocks.iter().any(|&count| count != 1)
    {
        return false;
    }
    for (index, block) in document.blocks.iter().enumerate() {
        if !span(block.span)
            || block.section >= document.sections.len()
            || block.parent.is_some_and(|parent| parent >= index)
        {
            return false;
        }
        for &child in &block.children {
            if document
                .blocks
                .get(child)
                .is_none_or(|value| value.parent != Some(index))
            {
                return false;
            }
            block_children[child] += 1;
        }
        for &sentence in &block.sentences {
            if document
                .sentences
                .get(sentence)
                .is_none_or(|value| value.block != index)
            {
                return false;
            }
            block_sentences[sentence] += 1;
        }
    }
    if document
        .blocks
        .iter()
        .zip(block_children)
        .any(|(block, count)| count != usize::from(block.parent.is_some()))
        || block_sentences.iter().any(|&count| count != 1)
    {
        return false;
    }
    for sentence in &document.sentences {
        if !span(sentence.span) || sentence.block >= document.blocks.len() {
            return false;
        }
        for fragment in &sentence.fragments {
            if !span(fragment.span) {
                return false;
            }
            let mut end = 0;
            for mapping in &fragment.mapping {
                if !span(mapping.source)
                    || mapping.text.start != end
                    || mapping.text.start >= mapping.text.end
                    || fragment
                        .text
                        .get(mapping.text.start..mapping.text.end)
                        .is_none()
                {
                    return false;
                }
                end = mapping.text.end;
            }
            if end != fragment.text.len() {
                return false;
            }
        }
    }
    document
        .links
        .iter()
        .all(|value| span(value.span) && span(value.destination_span))
        && document.comments.iter().all(|value| span(value.span))
        && document.identifiers.iter().all(|value| {
            span(value.span)
                && document
                    .blocks
                    .get(value.block)
                    .is_some_and(|block| block.section == value.section)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "---\nkind: reference\n---\n# 安装 Install\n\n- `value`: A &amp; B.\n\n[link](#安装-install)\n";

    #[test]
    fn cold_warm_disabled_are_equal_and_no_cache_does_no_io() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("cache");
        let cache = ParseCache::new(directory.clone(), true);
        let cold = cache.parse(SOURCE).unwrap();
        let warm = cache.parse(SOURCE).unwrap();
        assert_eq!(cold, warm);
        assert!(cache.read(&cache.entry_path(SOURCE), SOURCE).is_some());
        let absent = temporary.path().join("disabled");
        assert_eq!(
            cold,
            ParseCache::new(absent.clone(), false)
                .parse(SOURCE)
                .unwrap()
        );
        assert!(!absent.exists());
    }

    #[test]
    fn corrupt_bytes_and_decodable_models_with_bad_references_are_misses() {
        let temporary = tempfile::tempdir().unwrap();
        let cache = ParseCache::new(temporary.path().to_path_buf(), true);
        let expected = cache.parse(SOURCE).unwrap();
        let path = cache.entry_path(SOURCE);
        std::fs::write(&path, b"not cache data").unwrap();
        assert_eq!(cache.parse(SOURCE).unwrap(), expected);
        let mut corrupt = expected.clone();
        corrupt.sentences[0].block = usize::MAX;
        cache.write(&path, &corrupt);
        assert!(cache.read(&path, SOURCE).is_none());
        assert_eq!(cache.parse(SOURCE).unwrap(), expected);
        corrupt = expected.clone();
        corrupt.sections[corrupt.blocks[0].section].blocks.clear();
        cache.write(&path, &corrupt);
        assert!(cache.read(&path, SOURCE).is_none());
        assert_eq!(cache.parse(SOURCE).unwrap(), expected);
    }

    #[test]
    fn shared_models_reuse_content_without_retaining_documents() {
        let temporary = tempfile::tempdir().unwrap();
        let cache = ParseCache::new(temporary.path().to_path_buf(), true);
        let first = cache.parse_shared(SOURCE).unwrap();
        let second = cache.parse_shared(SOURCE).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let weak = Arc::downgrade(&first);
        drop(first);
        drop(second);
        assert!(weak.upgrade().is_none());
        assert_eq!(
            *cache.parse_shared(SOURCE).unwrap(),
            crate::md::parse(SOURCE).unwrap()
        );
        let disabled = ParseCache::new(temporary.path().join("disabled"), false);
        assert!(!Arc::ptr_eq(
            &disabled.parse_shared(SOURCE).unwrap(),
            &disabled.parse_shared(SOURCE).unwrap()
        ));
        assert!(!temporary.path().join("disabled").exists());
    }

    #[test]
    fn binary_payload_rejects_trailing_bytes_and_oversized_lengths() {
        let temporary = tempfile::tempdir().unwrap();
        let cache = ParseCache::new(temporary.path().to_path_buf(), true);
        let document = crate::md::parse(SOURCE).unwrap();
        let mut trailing = bincode::options().serialize(&document).unwrap();
        trailing.push(0);
        let oversized_string = bincode::options().serialize(&u64::MAX).unwrap();
        for payload in [trailing, oversized_string] {
            let mut bytes = HEADER.to_vec();
            bytes.extend_from_slice(&Sha256::digest(&payload));
            bytes.extend_from_slice(&payload);
            std::fs::write(cache.entry_path(SOURCE), bytes).unwrap();
            assert!(cache.read(&cache.entry_path(SOURCE), SOURCE).is_none());
            assert_eq!(cache.parse(SOURCE).unwrap(), document);
        }
    }

    #[test]
    fn changed_content_version_or_unwritable_directory_cannot_change_parse() {
        let temporary = tempfile::tempdir().unwrap();
        let cache = ParseCache::new(temporary.path().to_path_buf(), true);
        let document = cache.parse(SOURCE).unwrap();
        assert_ne!(cache.entry_path(SOURCE), cache.entry_path("# Other"));
        cache.write(&cache.entry_path("# Other"), &document);
        assert_eq!(
            cache.parse("# Other").unwrap(),
            crate::md::parse("# Other").unwrap()
        );
        let path = cache.entry_path(SOURCE);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[0] = b'X';
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(cache.parse(SOURCE).unwrap(), document);
        let blocker = temporary.path().join("file");
        std::fs::write(&blocker, "occupied").unwrap();
        assert_eq!(
            ParseCache::new(blocker, true).parse(SOURCE).unwrap(),
            document
        );
    }

    #[test]
    fn concurrent_readers_and_atomic_writers_return_the_same_document() {
        let temporary = tempfile::tempdir().unwrap();
        let cache = ParseCache::new(temporary.path().to_path_buf(), true);
        let expected = crate::md::parse(SOURCE).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..12 {
                let cache = &cache;
                let expected = &expected;
                scope.spawn(move || {
                    for _ in 0..8 {
                        assert_eq!(&cache.parse(SOURCE).unwrap(), expected);
                    }
                });
            }
        });
        assert!(cache.read(&cache.entry_path(SOURCE), SOURCE).is_some());
        let entries = std::fs::read_dir(temporary.path())
            .unwrap()
            .filter(|entry| {
                let name = entry.as_ref().unwrap().file_name();
                !matches!(
                    name.to_str(),
                    Some(".gitignore" | "CACHEDIR.TAG" | PRUNE_MARKER)
                )
            })
            .count();
        assert_eq!(entries, 1);
    }

    #[test]
    fn maintenance_ignores_the_directory_and_prunes_expired_entries_once_a_day() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("cache");
        let cache = ParseCache::new(directory.clone(), true);
        cache.parse(SOURCE).unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.join(".gitignore")).unwrap(),
            GITIGNORE
        );
        assert!(
            std::fs::read_to_string(directory.join("CACHEDIR.TAG"))
                .unwrap()
                .starts_with("Signature: 8a477f597d28d172789f06886806bc55")
        );
        let expired = directory.join(format!("{}.cache", "0".repeat(64)));
        let leftover = directory.join(".tmpABCDEF");
        let unrelated = directory.join("notes.txt");
        let now = SystemTime::now();
        for path in [&expired, &leftover, &unrelated] {
            std::fs::write(path, b"old").unwrap();
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(now - MAX_ENTRY_AGE - Duration::from_secs(60))
                .unwrap();
        }
        // The first write already swept the directory today.
        cache.maintain(now);
        assert!(expired.exists());
        std::fs::File::options()
            .write(true)
            .open(directory.join(PRUNE_MARKER))
            .unwrap()
            .set_modified(now - PRUNE_INTERVAL - Duration::from_secs(60))
            .unwrap();
        cache.maintain(now);
        assert!(!expired.exists());
        assert!(!leftover.exists());
        assert!(unrelated.exists());
        assert!(cache.read(&cache.entry_path(SOURCE), SOURCE).is_some());
    }

    #[test]
    #[ignore = "requires the downloaded corpus; run corpus/corpus.py fetch first"]
    fn pinned_corpus_cache_roundtrip() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let corpus = root.join("corpus");
        let lock_bytes = std::fs::read(corpus.join("corpus.lock.json")).unwrap();
        let lock: serde_json::Value = serde_json::from_slice(&lock_bytes).unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let cache = ParseCache::new(temporary.path().join("cache"), true);
        let disabled = ParseCache::new(temporary.path().join("disabled"), false);
        let started = std::time::Instant::now();
        let mut count = 0;
        let mut source_bytes = 0;
        for source in lock["sources"].as_array().unwrap() {
            for entry in source["documents"].as_array().unwrap() {
                let path = corpus
                    .join("data/blobs")
                    .join(entry["git_blob"].as_str().unwrap());
                let text = std::fs::read_to_string(&path).unwrap();
                assert_eq!(
                    format!("{:x}", Sha256::digest(text.as_bytes())),
                    entry["sha256"].as_str().unwrap()
                );
                let cold = cache.parse(&text).unwrap();
                let warm = cache
                    .read(&cache.entry_path(&text), &text)
                    .unwrap_or_else(|| panic!("cache rejected {}:{}", source["id"], entry["path"]));
                assert_eq!(cold, warm, "{}:{}", source["id"], entry["path"]);
                assert_eq!(
                    cold,
                    disabled.parse(&text).unwrap(),
                    "{}:{}",
                    source["id"],
                    entry["path"]
                );
                count += 1;
                source_bytes += text.len();
            }
        }
        assert!(!temporary.path().join("disabled").exists());
        let report = serde_json::json!({
            "corpus_lock_sha256": format!("{:x}", Sha256::digest(&lock_bytes)),
            "documents": count,
            "source_bytes": source_bytes,
            "cold_warm_disabled_equal": true,
            "cache_hits": count,
            "elapsed_seconds": started.elapsed().as_secs_f64(),
        });
        let output = root.join("target/cache-corpus-roundtrip.json");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("{report}");
    }
}

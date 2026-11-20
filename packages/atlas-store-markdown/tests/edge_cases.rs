//! Cross-cutting edge cases exercised through the public API.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use atlas_store_markdown::slug::{UNTITLED, derive_id, slugify};
use atlas_store_markdown::{Frontmatter, MarkdownRecord, MarkdownStore, Value};
use tempfile::TempDir;

fn record(title: &str, body: &str) -> MarkdownRecord {
    let mut frontmatter = Frontmatter::new();
    frontmatter.set("title", Value::str(title));
    MarkdownRecord::new(frontmatter, body)
}

#[test]
fn tricky_titles_round_trip_through_disk() {
    let dir = TempDir::new().expect("temp");
    let store = MarkdownStore::open(dir.path()).expect("open");
    let relative = Path::new("memories/topic/a.md");

    let titles = [
        "Note: keep this",
        "quote \" inside",
        "line\nbreak",
        "\u{8a18}\u{61b6} caf\u{e9} \u{1f600}",
        "trailing space ",
        "{braces}, commas",
    ];
    for title in titles {
        let original = record(title, "body");
        store.write_record(relative, &original).expect("write");
        let read = store.read_record(relative).expect("read");
        assert_eq!(read.frontmatter.get_str("title"), Some(title), "{title:?}");
        let bytes = store
            .read_record(relative)
            .expect("read")
            .to_bytes(relative)
            .expect("encode");
        let reparsed = MarkdownRecord::parse(&bytes, relative).expect("re-parse");
        assert_eq!(reparsed, read, "{title:?}: byte stability");
    }
}

#[test]
fn unicode_tags_and_topics_round_trip() {
    let dir = TempDir::new().expect("temp");
    let store = MarkdownStore::open(dir.path()).expect("open");
    let relative = Path::new("memories/architecture/a.md");
    let mut frontmatter = Frontmatter::new();
    frontmatter.set("topic", Value::str("\u{8a18}\u{61b6}"));
    frontmatter.set(
        "tags",
        Value::list([Value::str("ci/cd"), Value::str("\u{1f600}-emoji")]),
    );
    let original = MarkdownRecord::new(frontmatter, "");
    store.write_record(relative, &original).expect("write");
    let read = store.read_record(relative).expect("read");
    assert_eq!(read.frontmatter.get_str("topic"), Some("\u{8a18}\u{61b6}"));
    assert_eq!(
        read.frontmatter.get_list("tags").expect("tags").len(),
        2,
        "tags preserved"
    );
}

#[test]
fn body_edge_cases_survive_store_round_trip() {
    let dir = TempDir::new().expect("temp");
    let store = MarkdownStore::open(dir.path()).expect("open");
    let cases = [
        "---\nfrontmatter-shaped body\n---\n",
        "\u{feff}bom start\r\ncrlf line\r\n",
        &"x".repeat(10_000),
        "",
    ];
    for (index, body) in cases.iter().enumerate() {
        let relative = PathBuf::from(format!("memories/topic/{index}.md"));
        let original = record("x", body);
        store.write_record(&relative, &original).expect("write");
        let read = store.read_record(&relative).expect("read");
        assert_eq!(&read.body, body, "case {index}");
        let on_disk = std::fs::read(dir.path().join(&relative)).expect("raw");
        let reencoded = read.to_bytes(&relative).expect("encode");
        assert_eq!(on_disk, reencoded, "case {index}: byte-identical on disk");
    }
}

#[test]
fn slug_collisions_and_id_determinism() {
    assert_eq!(slugify("Hello, World!"), slugify("hello world"));
    assert_eq!(slugify("\u{8a18}\u{61b6}"), UNTITLED);
    assert_eq!(derive_id(&["a", "b"]), derive_id(&["a", "b"]));
    assert_ne!(derive_id(&["a", "b"]), derive_id(&["ab"]));
}

#[test]
fn concurrent_reader_sees_old_or_new_never_torn() {
    let dir = TempDir::new().expect("temp");
    let store = MarkdownStore::open(dir.path()).expect("open");
    let relative = PathBuf::from("memories/topic/live.md");
    store
        .write_record(&relative, &record("old", &"a".repeat(4096)))
        .expect("seed");

    let barrier = Arc::new(Barrier::new(3));

    let writer = {
        let store = store.clone();
        let relative = relative.clone();
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            for index in 0..200 {
                let (title, body) = if index % 2 == 0 {
                    ("new", "b".repeat(8192))
                } else {
                    ("old", "a".repeat(4096))
                };
                store
                    .write_record(&relative, &record(title, &body))
                    .expect("write");
            }
        })
    };

    let reader = {
        let store = store.clone();
        let relative = relative.clone();
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            for _ in 0..400 {
                let read = store.read_record(&relative).expect("read never fails");
                let title = read.frontmatter.get_str("title").expect("title");
                let expected = if title == "old" {
                    "a".repeat(4096)
                } else {
                    assert_eq!(title, "new");
                    "b".repeat(8192)
                };
                assert_eq!(read.body, expected, "torn read detected");
            }
        })
    };

    barrier.wait();
    writer.join().expect("writer");
    reader.join().expect("reader");
}

#[test]
fn stale_tmp_fragments_cleaned_on_open() {
    let dir = TempDir::new().expect("temp");
    let tmp = dir.path().join("tmp");
    std::fs::create_dir_all(&tmp).expect("tmp");
    let stale = tmp.join("999-0-1.tmp");
    std::fs::write(&stale, b"stale").expect("write");
    let old = std::time::SystemTime::now() - Duration::from_secs(2 * 60 * 60);
    std::fs::File::open(&stale)
        .expect("open")
        .set_modified(old)
        .expect("mtime");

    let _store = MarkdownStore::open(dir.path()).expect("open");
    assert!(!stale.exists(), "stale fragment must be cleaned on open");
}

#[test]
fn lock_release_is_bounded_after_drop() {
    let dir = TempDir::new().expect("temp");
    let store = MarkdownStore::open(dir.path()).expect("open");
    let lock =
        atlas_store_markdown::lock::StoreLock::acquire(store.root(), "memories").expect("lock");
    drop(lock);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if atlas_store_markdown::lock::StoreLock::try_acquire(store.root(), "memories")
            .expect("try")
            .is_some()
        {
            break;
        }
        assert!(Instant::now() < deadline, "lock not released");
        std::thread::yield_now();
    }
}

use apphost::logs::{LogWriter, Stream};
use std::fs;
use tempfile::TempDir;

#[test]
fn raw_binary_is_preserved_and_offsets_reconstruct_both_streams() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("instance");
    let mut writer = LogWriter::create(root.clone(), 64 * 1024, 2).unwrap();
    let out = vec![0, 255, 10, 128, 65];
    writer.append("web", Stream::Stdout, &out).unwrap();
    writer
        .append("web", Stream::Stderr, b"error without newline")
        .unwrap();
    writer.append("web", Stream::Stdout, b"tail").unwrap();
    drop(writer);
    let index = fs::read_to_string(root.join("0/index.jsonl")).unwrap();
    let records: Vec<serde_json::Value> = index
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 3);
    assert_eq!(records[2]["offset"], out.len() as u64);
    assert_eq!(records[2]["sequence"], 2);
    for record in &records {
        let suffix = if record["stream"] == "stdout" {
            "out"
        } else {
            "err"
        };
        let bytes = fs::read(root.join(format!("0/web.{suffix}"))).unwrap();
        let offset = record["offset"].as_u64().unwrap() as usize;
        let length = record["length"].as_u64().unwrap() as usize;
        assert!(offset + length <= bytes.len());
        assert!(record["observed_ms"].as_u64().unwrap() > 0);
    }
    let mut expected = out;
    expected.extend_from_slice(b"tail");
    assert_eq!(fs::read(root.join("0/web.out")).unwrap(), expected);
}

#[test]
fn rotation_bounds_raw_and_index_even_for_tiny_writes_and_long_lines() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("instance");
    let mut writer = LogWriter::create(root.clone(), 1024, 3).unwrap();
    for _ in 0..500 {
        writer.append("web", Stream::Stdout, b"x").unwrap();
    }
    writer
        .append("web", Stream::Stderr, &vec![255; 128 * 1024])
        .unwrap();
    drop(writer);
    let segments: Vec<_> = fs::read_dir(&root).unwrap().map(|e| e.unwrap()).collect();
    assert_eq!(segments.len(), 3);
    let mut total = 0;
    for segment in segments {
        let files: Vec<_> = fs::read_dir(segment.path())
            .unwrap()
            .map(|e| e.unwrap())
            .collect();
        let size: u64 = files.iter().map(|e| e.metadata().unwrap().len()).sum();
        assert!(size <= 1024, "segment size {size}");
        total += size;
        let index = fs::read_to_string(segment.path().join("index.jsonl")).unwrap();
        for line in index.lines() {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
    }
    assert!(total <= 3072);
}

#[test]
fn invalid_keys_budgets_existing_directories_and_rotation_io_fail_explicitly() {
    let temp = TempDir::new().unwrap();
    assert!(LogWriter::create(temp.path().join("bad"), 0, 1).is_err());
    assert!(!temp.path().join("bad").exists());
    let root = temp.path().join("instance");
    let mut writer = LogWriter::create(root.clone(), 1024, 2).unwrap();
    assert!(LogWriter::create(root.clone(), 1024, 2).is_err());
    for key in ["", "../escape", "a/b", "a\\b", ".", ".."] {
        assert!(writer.append(key, Stream::Stdout, b"x").is_err());
    }
    assert!(
        writer
            .append(&"a".repeat(65), Stream::Stdout, b"x")
            .is_err()
    );
    // A failed next-segment creation must surface, not silently discard output
    // or adopt files created by somebody else.
    fs::write(root.join("1"), b"occupied").unwrap();
    assert!(
        writer
            .append("web", Stream::Stdout, &vec![1; 4096])
            .is_err()
    );
    assert_eq!(fs::read(root.join("1")).unwrap(), b"occupied");
}

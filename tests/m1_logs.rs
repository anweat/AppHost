use apphost::logs::{LogWriter, Stream};
use std::fs;
use tempfile::TempDir;

#[test]
fn sequence_follow_handles_pagination_and_rotated_prefix_without_duplicates() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("follow");
    let mut writer = LogWriter::create(root.clone(), 8192, 2).unwrap();
    for _ in 0..8 {
        writer.append("web", Stream::Stdout, &[b'x'; 4096]).unwrap();
    }
    let first = apphost::logs::read_after(&root, None, 4096).unwrap();
    assert!(first.prefix_evicted);
    assert_eq!(first.chunks.len(), 1);
    let sequence = first.chunks[0].sequence;
    let next = apphost::logs::read_after(&root, Some(sequence), 4096).unwrap();
    assert!(!next.prefix_evicted);
    assert_eq!(next.chunks[0].sequence, sequence + 1);
    let missed = apphost::logs::read_after(&root, Some(0), 4096).unwrap();
    assert!(missed.prefix_evicted);
}

#[test]
fn bounded_tail_preserves_the_final_error_after_a_stdout_flood() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("tail");
    let mut writer = LogWriter::create(root.clone(), 1024 * 1024, 2).unwrap();
    for _ in 0..100 {
        writer
            .append("s0", Stream::Stdout, &vec![b'x'; 4096])
            .unwrap();
    }
    writer
        .append("s0", Stream::Stderr, b"ModuleNotFoundError: gradio")
        .unwrap();
    drop(writer);
    let tail = apphost::logs::read_tail(&root, 0, u128::MAX, 8192).unwrap();
    assert!(tail.truncated);
    assert!(tail.chunks.iter().map(|c| c.bytes.len()).sum::<usize>() <= 8192);
    assert_eq!(
        tail.chunks.last().unwrap().bytes,
        b"ModuleNotFoundError: gradio"
    );
}

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
    let window = apphost::logs::read_window(&root, 0, u128::MAX, 4096).unwrap();
    assert!(window.prefix_evicted);
    assert!(!window.sequence_gap);
    assert!(!window.incomplete_tail);
}

#[test]
fn observation_window_filters_time_preserves_binary_and_bounds_output() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("window");
    let mut writer = LogWriter::create(root.clone(), 65536, 2).unwrap();
    writer.append("web", Stream::Stdout, b"before").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    writer
        .append("web", Stream::Stderr, &[0, 255, 128])
        .unwrap();
    let all = apphost::logs::read_window(&root, 0, u128::MAX, 1024).unwrap();
    assert_eq!(all.chunks.len(), 2);
    let timestamp = all.chunks[1].observed_ms;
    assert!(timestamp > all.chunks[0].observed_ms);
    let window = apphost::logs::read_window(&root, timestamp, timestamp, 3).unwrap();
    assert_eq!(window.chunks.len(), 1);
    assert_eq!(window.chunks[0].bytes, [0, 255, 128]);
    assert!(!window.truncated);
    let limited = apphost::logs::read_window(&root, 0, u128::MAX, 5).unwrap();
    assert!(limited.truncated);
    assert!(limited.chunks.is_empty());
    assert!(apphost::logs::read_window(&root, 2, 1, 1024).is_err());
    assert!(apphost::logs::read_window(&root, 0, 1, 0).is_err());
    assert!(apphost::logs::read_window(&root, 0, 1, 1024 * 1024 + 1).is_err());
}

#[test]
fn partial_index_and_corrupt_offsets_are_not_reported_as_complete_logs() {
    use std::io::Write;
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("window");
    let mut writer = LogWriter::create(root.clone(), 65536, 2).unwrap();
    writer.append("web", Stream::Stdout, b"before").unwrap();
    drop(writer);
    let index = root.join("0/index.jsonl");
    let original = fs::read(&index).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&index)
        .unwrap()
        .write_all(b"{\"sequence\":")
        .unwrap();
    let window = apphost::logs::read_window(&root, 0, u128::MAX, 1024).unwrap();
    assert!(window.incomplete_tail);
    assert_eq!(window.chunks[0].bytes, b"before");
    let mut record: serde_json::Value = serde_json::from_slice(&original).unwrap();
    for (field, value) in [
        ("offset", serde_json::json!(u64::MAX)),
        ("service_key", serde_json::json!("../outside")),
        ("segment", serde_json::json!(1)),
    ] {
        let mut corrupt = record.clone();
        corrupt[field] = value;
        fs::write(&index, format!("{corrupt}\n")).unwrap();
        assert!(apphost::logs::read_window(&root, 0, u128::MAX, 1024).is_err());
    }
    record["sequence"] = serde_json::json!(5);
    record["observed_ms"] = serde_json::json!(0);
    fs::write(
        &index,
        [&original, format!("{record}\n").as_bytes()].concat(),
    )
    .unwrap();
    let window = apphost::logs::read_window(&root, 0, u128::MAX, 1024).unwrap();
    assert!(window.sequence_gap);
    assert!(window.clock_reversed);
    fs::write(&index, vec![b'a'; 1025]).unwrap();
    assert!(apphost::logs::read_window(&root, 0, u128::MAX, 1024).is_err());
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
    fs::remove_file(root.join("1")).unwrap();
    assert!(
        writer
            .append("web", Stream::Stdout, b"no hidden retry")
            .is_err()
    );
}

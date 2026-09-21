use apphost::{Manifest, scan};
use serde::Deserialize;
use std::{env, fs, path::PathBuf};

#[derive(Deserialize)]
struct Case {
    root: PathBuf,
    manifest: Option<Manifest>,
    manifest_file: Option<PathBuf>,
    expected_candidates: usize,
}

// Opt-in local acceptance check. The case file stays outside the repository;
// source projects are only read, never registered, modified or launched.
#[test]
#[ignore = "Set APPHOST_REAL_PROJECT_CASES to a JSON array of at least three local app declarations"]
fn real_projects_read_only() {
    let path =
        env::var_os("APPHOST_REAL_PROJECT_CASES").expect("APPHOST_REAL_PROJECT_CASES is required");
    let cases: Vec<Case> = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert!(cases.len() >= 3);
    for case in cases {
        let manifest = match (case.manifest, case.manifest_file) {
            (Some(manifest), None) => manifest,
            (None, Some(path)) => {
                toml::from_str::<Manifest>(&fs::read_to_string(path).unwrap()).unwrap()
            }
            _ => panic!("Provide exactly one manifest or manifest_file"),
        };
        let issues = manifest.validate(&case.root);
        assert!(issues.is_empty(), "{}: {issues:?}", case.root.display());
        let encoded = toml::to_string_pretty(&manifest).unwrap();
        let decoded: Manifest = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded, manifest);
        let draft = scan(&case.root, 0).unwrap();
        assert_eq!(draft.candidates.len(), case.expected_candidates);
        assert!(draft.issues.is_empty(), "{:?}", draft.issues);
        let warnings = manifest.launch_warnings(&case.root);
        println!(
            "{}: {} services validated; {} scan candidates; {} selected",
            case.root.display(),
            manifest.services.len(),
            draft.candidates.len(),
            draft.candidates.iter().filter(|c| c.selected).count()
        );
        println!("Static launch warnings (no execution): {warnings:?}");
    }
}

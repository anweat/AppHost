use apphost::{
    discovery,
    rules::grouping::{CONVENTIONAL_GROUPING, GroupingRule},
    *,
};
use std::{collections::BTreeSet, fs, io::Cursor, path::Path};
use tempfile::TempDir;

fn package(root: &Path, value: serde_json::Value) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("package.json"), value.to_string()).unwrap();
}
fn node(root: &Path) {
    package(root, serde_json::json!({"scripts":{"dev":"vite"}}));
}

#[test]
fn mixed_case_pairs_preserve_paths_and_confirm_one_multiservice_app() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("app");
    node(&root.join("Frontend"));
    fs::create_dir_all(root.join("BACKEND")).unwrap();
    fs::write(root.join("BACKEND/requirements.txt"), "flask").unwrap();
    let mut draft = scan(temp.path(), 2).unwrap();
    assert_eq!(draft.groups.len(), 1);
    assert!(draft.candidates.iter().all(|c| !c.selected));
    review::review(
        &mut draft,
        &mut Cursor::new(b"y\n[\"flask\",\"run\"]\n"),
        &mut vec![],
    )
    .unwrap();
    let imported = import(&temp.path().join("data"), &draft).unwrap();
    assert_eq!(imported.len(), 1);
    let manifest = validate(&root).unwrap();
    assert_eq!(
        manifest
            .services
            .iter()
            .map(|s| s.cwd.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["Frontend", "BACKEND"])
    );
}

#[test]
fn wrapped_service_members_are_warned_without_merging_or_deselecting_docs() {
    for wrapped_backend in [false, true] {
        for collection_root in [false, true] {
            let temp = TempDir::new().unwrap();
            let root = temp.path().join("app");
            node(&root.join("Frontend/Vue"));
            let backend = root.join(if wrapped_backend {
                "Backend/Python"
            } else {
                "Backend"
            });
            fs::create_dir_all(&backend).unwrap();
            fs::write(backend.join("requirements.txt"), "flask").unwrap();
            node(&root.join("docs"));
            let mut draft = scan(if collection_root { &root } else { temp.path() }, 3).unwrap();
            assert!(draft.groups.is_empty());
            assert_eq!(draft.candidates.len(), 3);
            for c in &draft.candidates {
                if c.path.ends_with("docs") {
                    assert!(c.selected);
                } else {
                    assert!(!c.selected, "{}", c.path.display());
                    assert!(
                        c.notes
                            .iter()
                            .any(|n| n.field == "group" && n.message.contains("apphost.toml"))
                    );
                }
            }
            review::review(&mut draft, &mut Cursor::new(b"n\nn\n"), &mut vec![]).unwrap();
            let imported = import(&temp.path().join("data"), &draft).unwrap();
            assert_eq!(imported.len(), 1);
            assert!(imported[0].path.ends_with("docs"));
        }
    }
}

#[test]
fn case_collisions_remain_ambiguous_and_rules_do_not_invent_paths() {
    let names = BTreeSet::from(["Frontend".into(), "frontend".into(), "Backend".into()]);
    assert!(CONVENTIONAL_GROUPING.sibling_pair(&names).is_none());
    let pairs = CONVENTIONAL_GROUPING.sibling_pairs(&names);
    assert_eq!(pairs.len(), 2);
    assert!(pairs.iter().flatten().all(|n| names.contains(n)));
    assert!(!CONVENTIONAL_GROUPING.is_service_directory("frontend-extra"));
}

#[test]
fn library_and_build_watch_commands_require_confirmation_but_keep_the_proposal() {
    for metadata in [
        serde_json::json!({"exports":"./dist/index.js", "scripts":{"dev":"vite"}}),
        serde_json::json!({"main":"dist/index.js", "files":["dist"], "scripts":{"start":"node index.js"}}),
        serde_json::json!({"scripts":{"dev":"tsc -w"}}),
        serde_json::json!({"scripts":{"dev":"tsdown --watch"}}),
        serde_json::json!({"scripts":{"dev":"rollup --watch=true"}}),
    ] {
        let temp = TempDir::new().unwrap();
        package(temp.path(), metadata);
        let draft = scan(temp.path(), 0).unwrap();
        let c = &draft.candidates[0];
        assert!(!c.selected);
        assert!(c.notes.iter().any(|n| n.field == "launch"));
        assert!(!c.manifest.as_ref().unwrap().services[0].command.is_empty());
        assert!(
            import(&temp.path().join("data"), &draft)
                .unwrap()
                .is_empty()
        );
        let mut reviewed = draft;
        review::review(&mut reviewed, &mut Cursor::new(b"y\n\n"), &mut vec![]).unwrap();
        assert_eq!(
            import(&temp.path().join("data"), &reviewed).unwrap().len(),
            1
        );
    }
}

#[test]
fn server_watchers_main_only_and_authoritative_manifests_keep_their_boundaries() {
    for script in [
        "vite",
        "next dev",
        "webpack-dev-server --open",
        "node --watch server.js",
        "nodemon server.js",
    ] {
        let temp = TempDir::new().unwrap();
        package(
            temp.path(),
            serde_json::json!({"main":"index.js", "scripts":{"dev":script}}),
        );
        let draft = scan(temp.path(), 0).unwrap();
        assert!(draft.candidates[0].selected, "{script}");
        let manifest = draft.candidates[0].manifest.as_ref().unwrap();
        fs::write(
            temp.path().join("apphost.toml"),
            manifest.to_toml().unwrap(),
        )
        .unwrap();
        package(
            temp.path(),
            serde_json::json!({"exports":"./index.js", "scripts":{"dev":"tsc -w"}}),
        );
        let owned = scan(temp.path(), 0).unwrap();
        assert!(owned.candidates[0].selected);
        assert!(owned.candidates[0].suggestions.is_empty());
    }
    // The wrapper guard stays within the requested tree depth.
    let temp = TempDir::new().unwrap();
    node(&temp.path().join("app/Frontend/Vue"));
    fs::create_dir_all(temp.path().join("app/Backend")).unwrap();
    assert!(
        discovery::scan(temp.path(), 1)
            .unwrap()
            .candidates
            .is_empty()
    );
}

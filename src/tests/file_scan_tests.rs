use rig::tool::PortableTool;

use crate::agent::tools::{
    FindFilesArgs, FindFilesTool, GrepArgs, GrepTool, ListDirArgs, ListDirTool,
};

#[tokio::test]
async fn scans_keep_filters_limits_and_workspace_on_workers() {
    let root = std::env::temp_dir().join(format!("zs-scans-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::write(root.join("ignored.txt"), "needle\n").unwrap();
    std::fs::write(root.join("target/generated.rs"), "needle\n").unwrap();
    std::fs::write(root.join("src/a.rs"), "before\nneedle\nafter\n").unwrap();
    std::fs::write(root.join("src/b.rs"), "needle\n").unwrap();
    std::fs::write(root.join("binary.rs"), b"\0needle").unwrap();
    let owner = crate::engine::resources::Resources::for_workspace(root.clone());
    owner
        .clone()
        .scope(async {
            let found = FindFilesTool::new(None, None, 100)
                .call(FindFilesArgs {
                    pattern: "a\\.rs$".into(),
                    path: Some(".".into()),
                })
                .await
                .unwrap();
            let path = found
                .lines()
                .find(|line| line.ends_with("/src/a.rs"))
                .expect(&found);
            assert_eq!(std::fs::canonicalize(path).unwrap(), root.join("src/a.rs"));
            assert!(!found.contains("generated.rs"));
            let found = FindFilesTool::new(None, None, 1)
                .call(FindFilesArgs {
                    pattern: "\\.rs$".into(),
                    path: Some("src".into()),
                })
                .await
                .unwrap();
            assert!(found.contains("showing first 1"), "{found}");

            let grep = GrepTool::new(None, None, 100)
                .call(GrepArgs {
                    pattern: "needle".into(),
                    path: None,
                    include: None,
                    context_lines: Some(1),
                })
                .await
                .unwrap();
            assert!(grep.contains("before") && grep.contains("after"), "{grep}");
            assert!(
                !grep.contains("ignored.txt")
                    && !grep.contains("generated.rs")
                    && !grep.contains("binary.rs"),
                "{grep}"
            );
            let grep = GrepTool::new(None, None, 1)
                .call(GrepArgs {
                    pattern: "needle".into(),
                    path: None,
                    include: Some("*.rs".into()),
                    context_lines: None,
                })
                .await
                .unwrap();
            assert!(grep.contains("showing first 1"), "{grep}");

            let listing = ListDirTool::new(None, None, Some(1))
                .call(ListDirArgs { path: None })
                .await
                .unwrap();
            assert!(listing.contains("[dir(2)]  src"), "{listing}");
            assert!(listing.contains("truncated after 1 entries"), "{listing}");
            assert!(
                !listing.contains("ignored.txt") && !listing.contains("target"),
                "{listing}"
            );
        })
        .await;
    owner.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn invalid_patterns_are_reported_by_scan_tools() {
    let found = FindFilesTool::new(None, None, 1)
        .call(FindFilesArgs {
            pattern: "[".into(),
            path: None,
        })
        .await
        .unwrap_err();
    assert!(found.to_string().contains("Invalid regex"));
    let grep = GrepTool::new(None, None, 1)
        .call(GrepArgs {
            pattern: "[".into(),
            path: None,
            include: None,
            context_lines: None,
        })
        .await
        .unwrap_err();
    assert!(grep.to_string().contains("Invalid regex"));
}

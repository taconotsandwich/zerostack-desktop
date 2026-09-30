use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const PREVIEW_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub(super) struct Change {
    pub path: PathBuf,
    pub previous: Option<PathBuf>,
    pub status: String,
}

impl Change {
    pub fn label(&self) -> &'static str {
        if self.status.contains('U') || matches!(self.status.as_str(), "AA" | "DD") {
            "Conflict"
        } else if self.status.contains('R') {
            "Renamed"
        } else if self.status.contains('D') {
            "Deleted"
        } else if self.status == "??" || self.status.contains('A') {
            "Added"
        } else {
            "Modified"
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Repository {
    pub root: PathBuf,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone)]
pub(super) struct Preview {
    pub path: PathBuf,
    pub text: String,
    pub diff: bool,
}

pub(super) fn changes(project: &Path) -> Result<Repository, String> {
    let root = git(project, &["rev-parse", "--show-toplevel"])
        .map_err(|_| "Git changes are unavailable for this folder.".to_string())?;
    let root = path_from_bytes(root.strip_suffix(b"\n").unwrap_or(&root))?;
    let output = git(
        &root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    Ok(Repository {
        root,
        changes: parse_status(&output)?,
    })
}

fn parse_status(output: &[u8]) -> Result<Vec<Change>, String> {
    let mut records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    let mut changes = Vec::new();
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            return Err("Could not read Git status.".into());
        }
        let path = path_from_bytes(&record[3..])?;
        let renamed = record[..2].iter().any(|code| matches!(code, b'R' | b'C'));
        let previous = if renamed {
            Some(path_from_bytes(
                records
                    .next()
                    .ok_or("Missing rename source in Git status.")?,
            )?)
        } else {
            None
        };
        changes.push(Change {
            path,
            previous,
            status: String::from_utf8_lossy(&record[..2]).into_owned(),
        });
    }
    Ok(changes)
}

pub(super) fn diff(root: &Path, change: &Change) -> Result<Preview, String> {
    let path = root.join(&change.path);
    if change.status == "??" {
        let file = file(&path)?;
        return Ok(Preview {
            text: format!("New file\n\n{}", file.text),
            diff: true,
            ..file
        });
    }
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > PREVIEW_BYTES) {
        return Ok(Preview {
            path,
            text: "File exceeds the 1 MiB preview limit.".into(),
            diff: true,
        });
    }
    let has_head = git(root, &["rev-parse", "--verify", "HEAD"]).is_ok();
    let mut command = git_command(root);
    command.args(["diff", "--no-ext-diff", "--no-textconv", "--no-color"]);
    if has_head {
        command.arg("HEAD");
    } else {
        command.arg("--cached");
    }
    command.arg("--").arg(&change.path);
    if let Some(previous) = &change.previous {
        command.arg(previous);
    }
    let output = command.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !has_head {
        let output = git_command(root)
            .args(["diff", "--no-ext-diff", "--no-textconv", "--no-color", "--"])
            .arg(&change.path)
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().into());
        }
        if !output.stdout.is_empty() {
            text.push_str("\nUnstaged changes\n");
            text.push_str(&String::from_utf8_lossy(&output.stdout));
        }
    }
    if text.is_empty() {
        text = "No content differences. The file may have changed since refresh.".into();
    }
    if text.len() > PREVIEW_BYTES as usize {
        let mut boundary = PREVIEW_BYTES as usize;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        text.truncate(boundary);
        text.push_str("\n\nPreview truncated at 1 MiB.");
    }
    Ok(Preview {
        path,
        text,
        diff: true,
    })
}

pub(super) fn file(path: &Path) -> Result<Preview, String> {
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    let file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("Select a regular file.".into());
    }
    let mut bytes = Vec::new();
    file.take(PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let text = if bytes.len() > PREVIEW_BYTES as usize {
        "File exceeds the 1 MiB preview limit.".into()
    } else if bytes.contains(&0) {
        "Binary file — text preview unavailable.".into()
    } else {
        String::from_utf8(bytes).unwrap_or_else(|_| "This file is not UTF-8 text.".into())
    };
    Ok(Preview {
        path,
        text,
        diff: false,
    })
}

pub(super) fn local_link(project: &Path, link: &str) -> Result<Option<PathBuf>, String> {
    let base =
        reqwest::Url::from_directory_path(project).map_err(|_| "Invalid project directory.")?;
    let url = base.join(link).map_err(|error| error.to_string())?;
    match url.scheme() {
        "http" | "https" => Ok(None),
        "file" => url
            .to_file_path()
            .map(Some)
            .map_err(|_| "This file URL cannot be opened locally.".into()),
        _ => Err("Only local files and web links can be opened.".into()),
    }
}

pub(super) fn open_file(path: &Path) -> Result<(), String> {
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer");
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut command = Command::new("xdg-open");
    let status = command
        .arg(path)
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("Could not open the file in its default application.".into())
    }
}

fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("--literal-pathspecs");
    command
}

fn git(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    let output = git_command(root)
        .args(arguments)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().into())
    }
}

fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf, String> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(OsString::from_vec(bytes.to_vec()).into())
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes.to_vec())
            .map(PathBuf::from)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_preserves_spaces_newlines_and_rename_pairs() {
        let entries =
            parse_status(b" M src/with space.rs\0R  new\nname.rs\0old name.rs\0?? :(glob)*.txt\0")
                .unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path, Path::new("src/with space.rs"));
        assert_eq!(
            entries[1].previous.as_deref(),
            Some(Path::new("old name.rs"))
        );
        assert_eq!(entries[1].path, Path::new("new\nname.rs"));
    }

    #[test]
    fn links_distinguish_local_paths_from_web_urls() {
        assert_eq!(
            local_link(Path::new("/work"), "src/a%20b.rs#L12").unwrap(),
            Some(PathBuf::from("/work/src/a b.rs"))
        );
        assert_eq!(
            local_link(Path::new("/work"), "https://example.com").unwrap(),
            None
        );
        assert!(local_link(Path::new("/work"), "javascript:alert(1)").is_err());
    }

    #[test]
    fn review_reads_staged_unstaged_deleted_and_new_files_without_changing_git() {
        let root = std::env::temp_dir().join(format!("zs-review-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q"]).unwrap();
        git(&root, &["config", "user.name", "Desktop test"]).unwrap();
        git(&root, &["config", "user.email", "desktop@example.invalid"]).unwrap();
        std::fs::write(root.join("file.txt"), "before\n").unwrap();
        std::fs::write(root.join("deleted.txt"), "removed\n").unwrap();
        git(&root, &["add", "."]).unwrap();
        git(
            &root,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-qm",
                "initial",
                "--no-gpg-sign",
            ],
        )
        .unwrap();
        std::fs::write(root.join("file.txt"), "staged\n").unwrap();
        git(&root, &["add", "file.txt"]).unwrap();
        std::fs::write(root.join("file.txt"), "working\n").unwrap();
        std::fs::remove_file(root.join("deleted.txt")).unwrap();
        std::fs::write(root.join("new file.txt"), "new content\n").unwrap();
        let before = git(&root, &["status", "--porcelain=v1", "-z"]).unwrap();
        let repo = changes(&root).unwrap();
        for change in &repo.changes {
            let preview = diff(&repo.root, change).unwrap();
            match change.path.to_str().unwrap() {
                "file.txt" => {
                    assert!(preview.text.contains("-before"));
                    assert!(preview.text.contains("+working"));
                }
                "deleted.txt" => assert!(preview.text.contains("-removed")),
                "new file.txt" => assert!(preview.text.contains("new content")),
                path => panic!("unexpected {path}"),
            }
        }
        assert_eq!(
            before,
            git(&root, &["status", "--porcelain=v1", "-z"]).unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

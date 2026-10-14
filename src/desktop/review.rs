use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
const PREVIEW_BYTES: u64 = 1024 * 1024;
#[derive(Debug, Clone)]
pub(super) struct Preview {
    pub path: PathBuf,
    pub text: String,
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
    Ok(Preview { path, text })
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

#[cfg(test)]
mod tests {
    use super::*;
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
    fn file_previews_detect_binary_and_limit_large_files() {
        let root = std::env::temp_dir().join(format!("zs-file-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("preview");
        std::fs::write(&path, [0, 1, 2]).unwrap();
        assert!(file(&path).unwrap().text.contains("Binary"));
        std::fs::write(&path, vec![b'x'; PREVIEW_BYTES as usize + 1]).unwrap();
        assert!(file(&path).unwrap().text.contains("limit"));
        std::fs::remove_dir_all(root).unwrap();
    }
}

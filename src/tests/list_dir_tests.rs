use crate::agent::tools::list_dir::format_size;

#[test]
fn format_0_bytes() {
    assert_eq!(format_size(0), "0 B");
}

#[test]
fn format_1_byte() {
    assert_eq!(format_size(1), "1 B");
}

#[test]
fn format_1023_bytes() {
    assert_eq!(format_size(1023), "1023 B");
}

#[test]
fn format_1024_bytes_1_kb() {
    assert_eq!(format_size(1024), "1.0 KB");
}

#[test]
fn format_1536_bytes() {
    assert_eq!(format_size(1536), "1.5 KB");
}

#[test]
fn format_1048576_bytes_1_mb() {
    assert_eq!(format_size(1048576), "1.0 MB");
}

#[test]
fn format_1073741824_bytes_1_gb() {
    assert_eq!(format_size(1073741824), "1.0 GB");
}

#[test]
fn format_above_gb_remains_gb() {
    // 2 TB
    assert_eq!(format_size(2_199_023_255_552), "2048.0 GB");
}

#[test]
fn format_512_bytes() {
    assert_eq!(format_size(512), "512 B");
}

#[test]
fn format_2048_bytes() {
    assert_eq!(format_size(2048), "2.0 KB");
}

#[test]
fn format_2560_bytes() {
    assert_eq!(format_size(2560), "2.5 KB");
}

#[test]
fn format_large_kb() {
    assert_eq!(format_size(1_047_552), "1023.0 KB");
}

use crate::agent::tools::list_dir::count_dir_entries;

#[tokio::test]
async fn count_empty_dir() {
    let dir = std::env::temp_dir().join(format!("zs_empty_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.clone();
    let count =
        crate::engine::blocking::run_cancellable(move |cancel| count_dir_entries(&path, &cancel))
            .await
            .unwrap();
    assert_eq!(count, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn count_dir_with_files() {
    let dir = std::env::temp_dir().join(format!("zs_files_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), b"a").unwrap();
    std::fs::write(dir.join("b.txt"), b"b").unwrap();
    let path = dir.clone();
    let count =
        crate::engine::blocking::run_cancellable(move |cancel| count_dir_entries(&path, &cancel))
            .await
            .unwrap();
    assert_eq!(count, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn count_nonexistent_dir() {
    assert_eq!(
        crate::engine::blocking::run_cancellable(|cancel| count_dir_entries(
            std::path::Path::new("/nonexistent_xyz_test"),
            &cancel
        ))
        .await
        .unwrap(),
        0
    );
}

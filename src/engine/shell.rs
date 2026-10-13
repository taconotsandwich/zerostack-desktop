//! Host shell execution with an explicit workspace and cancellation lifetime.
use std::process::{Output, Stdio};

pub(crate) async fn run(command: &str) -> std::io::Result<Output> {
    let mut cmd = tokio::process::Command::new("bash");
    cmd.current_dir(super::resources::working_dir()?)
        .args(["-c", command])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::sandbox::configure_child_lifetime(&mut cmd);
    let child = cmd.spawn()?;
    let mut guard = crate::sandbox::ProcessGroupGuard::new(child.id(), Default::default());
    let output = child.wait_with_output().await?;
    guard.disarm();
    Ok(output)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn shell_uses_its_owner_workspace() {
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let owner = super::super::resources::Resources::for_workspace(dir.clone());
        let output = owner.scope(super::run("pwd")).await.unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            dir.to_string_lossy()
        );
    }
}

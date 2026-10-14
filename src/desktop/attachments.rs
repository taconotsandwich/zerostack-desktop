use super::app::{App, Message, Panel};
use super::{components, layout, style, worker::Operation};
use iced::widget::{column, image, text};
use iced::{Element, Task};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(super) enum Event {
    Paste(bool),
    Pasted(String, bool, Result<PathBuf, String>),
    Prepared(String, Vec<PathBuf>, Vec<(PathBuf, image::Handle)>),
    Preview(PathBuf),
}
const MAX_PIXELS: usize = 16 * 1024 * 1024;
fn clipboard_image(directory: &Path) -> Result<PathBuf, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let image = clipboard
        .get_image()
        .map_err(|_| "No image is available on the clipboard.".to_string())?;
    save_rgba(directory, image.width, image.height, &image.bytes)
}
fn save_rgba(
    directory: &Path,
    width: usize,
    height: usize,
    pixels: &[u8],
) -> Result<PathBuf, String> {
    let count = width
        .checked_mul(height)
        .filter(|n| *n > 0 && *n <= MAX_PIXELS)
        .ok_or("Clipboard image exceeds the 16 megapixel limit.")?;
    if pixels.len() != count * 4 {
        return Err("Invalid clipboard image.".into());
    }
    let image = ::image::RgbaImage::from_raw(width as u32, height as u32, pixels.to_vec())
        .ok_or("Invalid clipboard image.")?;
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let path = directory.join(format!("{}.png", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|e| e.to_string())?;
    if let Err(error) =
        ::image::DynamicImage::ImageRgba8(image).write_to(&mut file, ::image::ImageFormat::Png)
    {
        let _ = std::fs::remove_file(&path);
        return Err(error.to_string());
    }
    Ok(path)
}
fn thumbnail(path: &Path) -> Option<image::Handle> {
    if std::fs::metadata(path).ok()?.len() > 32 * 1024 * 1024 {
        return None;
    }
    let mut reader = ::image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut limits = ::image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(1024, 1024).to_rgba8();
    Some(image::Handle::from_rgba(
        image.width(),
        image.height(),
        image.into_raw(),
    ))
}
impl App {
    pub(super) fn prepare_attachments(&self, paths: Vec<PathBuf>) -> Task<Message> {
        let Some(snapshot) = &self.snapshot else {
            return Task::none();
        };
        let id = snapshot.session.id.to_string();
        Task::perform(
            async move {
                let thumbnails = tokio::task::spawn_blocking({
                    let paths = paths.clone();
                    move || {
                        paths
                            .iter()
                            .filter_map(|p| thumbnail(p).map(|image| (p.clone(), image)))
                            .collect()
                    }
                })
                .await
                .unwrap_or_default();
                (id, paths, thumbnails)
            },
            |(id, paths, thumbnails)| Message::Attachment(Event::Prepared(id, paths, thumbnails)),
        )
    }
    pub(super) fn update_attachment(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Paste(quiet)
                if !self.busy && self.snapshot.is_some() && self.panel.is_none() =>
            {
                let id = self.snapshot.as_ref().unwrap().session.id.to_string();
                let directory = crate::session::storage::data_dir().join("attachments");
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || clipboard_image(&directory))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    move |result| Message::Attachment(Event::Pasted(id.clone(), quiet, result)),
                );
            }
            Event::Pasted(id, quiet, result)
                if self
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.session.id.as_str() == id) =>
            {
                match result {
                    Ok(path) => return self.prepare_attachments(vec![path]),
                    Err(error) if !quiet => self.error = error,
                    _ => {}
                }
            }
            Event::Prepared(id, paths, thumbnails)
                if self
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.session.id.as_str() == id)
                    && !self.busy =>
            {
                // Only retain previews for this conversation's current files.
                if let Some(snapshot) = &self.snapshot {
                    self.thumbnails.retain(|p, _| snapshot.files.contains(p));
                }
                self.thumbnails.extend(thumbnails);
                return self.dispatch(Operation::AddContextFiles(paths));
            }
            Event::Preview(path) => {
                if self.thumbnails.contains_key(&path) {
                    self.panel = Some(Panel::Attachment(path));
                } else {
                    return self.update_review(super::review_view::Event::Open(path));
                }
            }
            _ => {}
        }
        Task::none()
    }
    pub(super) fn attachment_view<'a>(&'a self, path: &'a Path) -> Element<'a, Message> {
        let mut content =
            column![text(path.display().to_string()).size(style::LABEL)].spacing(layout::MD);
        if let Some(handle) = self.thumbnails.get(path) {
            content = content.push(image(handle.clone()).height(400));
        }
        content
            .push(components::action(
                "Remove attachment",
                (!self.busy).then_some(Message::Operate(Operation::DropContextFile {
                    path: path.into(),
                })),
            ))
            .into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_images_are_lossless_private_and_previewable() {
        let directory =
            std::env::temp_dir().join(format!("zs-attachment-{}", uuid::Uuid::new_v4()));
        assert!(save_rgba(&directory, usize::MAX, 2, &[]).is_err());
        assert!(save_rgba(&directory, 1, 1, &[1, 2, 3]).is_err());
        let path = save_rgba(&directory, 1, 1, &[1, 2, 3, 255]).unwrap();
        assert_eq!(
            ::image::open(&path).unwrap().to_rgba8().into_raw(),
            [1, 2, 3, 255]
        );
        assert!(thumbnail(&path).is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

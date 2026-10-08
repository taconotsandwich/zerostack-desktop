//! The content blocks of an ACP prompt as zerostack input: text for the
//! message, media attached to it.

use agent_client_protocol::schema::v1::*;

#[cfg(feature = "multimodal")]
use crate::extras::multimodal::MediaAttachment;

/// A prompt read from its content blocks.
#[derive(Default)]
pub(super) struct Prompt {
    pub(super) text: String,
    #[cfg(feature = "multimodal")]
    pub(super) media: Vec<MediaAttachment>,
}

/// What a prompt may hold beyond text and resource links: images and audio
/// when zerostack is built with media support, and embedded resources.
pub(super) fn prompt_capabilities() -> PromptCapabilities {
    PromptCapabilities::new()
        .image(cfg!(feature = "multimodal"))
        .audio(cfg!(feature = "multimodal"))
        .embedded_context(true)
}

/// Read a prompt's blocks: text joins the message, a resource link is named
/// in it, an embedded text resource is quoted in it, and media is attached.
/// A block zerostack cannot take is an error naming it.
pub(super) fn read_prompt(blocks: Vec<ContentBlock>) -> Result<Prompt, String> {
    let mut prompt = Prompt::default();
    let mut parts = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text(text) => parts.push(text.text),
            ContentBlock::ResourceLink(link) => {
                parts.push(format!("[{}]({})", link.name, link.uri))
            }
            ContentBlock::Resource(resource) => match resource.resource {
                EmbeddedResourceResource::TextResourceContents(file) => parts.push(format!(
                    "<file uri=\"{}\">\n{}\n</file>",
                    file.uri, file.text
                )),
                EmbeddedResourceResource::BlobResourceContents(blob) => {
                    let mime = blob.mime_type.unwrap_or_default();
                    attach(&mut prompt, &blob.uri, &blob.blob, mime)?;
                }
                _ => return Err("unsupported embedded resource".to_string()),
            },
            ContentBlock::Image(image) => {
                let uri = image.uri.unwrap_or_else(|| "image".to_string());
                attach(&mut prompt, &uri, &image.data, image.mime_type)?;
            }
            ContentBlock::Audio(audio) => {
                attach(&mut prompt, "audio", &audio.data, audio.mime_type)?;
            }
            _ => return Err("unsupported content block".to_string()),
        }
    }
    prompt.text = parts.join("\n");
    Ok(prompt)
}

/// Attach base64 `data` of type `mime` to the prompt.
#[cfg(feature = "multimodal")]
fn attach(prompt: &mut Prompt, uri: &str, data: &str, mime: String) -> Result<(), String> {
    use base64::Engine as _;

    let supported = mime.starts_with("image/")
        || mime.starts_with("audio/")
        || (cfg!(feature = "pdf") && mime == "application/pdf");
    if !supported {
        return Err(format!("unsupported media type '{mime}' ({uri})"));
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| format!("invalid base64 in {uri}: {e}"))?;
    if data.len() as u64 > crate::extras::multimodal::MAX_MEDIA_BYTES {
        return Err(format!(
            "{uri} is too large ({} bytes, max {})",
            data.len(),
            crate::extras::multimodal::MAX_MEDIA_BYTES
        ));
    }
    let path = uri.strip_prefix("file://").unwrap_or(uri).into();
    prompt.media.push(MediaAttachment::new(path, data, mime));
    Ok(())
}

#[cfg(not(feature = "multimodal"))]
fn attach(_: &mut Prompt, uri: &str, _: &str, mime: String) -> Result<(), String> {
    Err(format!(
        "media ('{mime}', {uri}) needs zerostack built with the multimodal feature"
    ))
}

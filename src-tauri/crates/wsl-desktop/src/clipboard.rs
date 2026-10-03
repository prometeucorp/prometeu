//! Native clipboard adaptation. Existing attachment UI consumes execution-side paths.
pub struct Clipboard(pub std::path::PathBuf);

#[cfg(windows)]
impl prometeu_bridge::application::AttachmentClipboard for Clipboard {
    fn files(&self) -> Result<Vec<String>, String> {
        let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        match clipboard.get().file_list() {
            Ok(files) if !files.is_empty() => {
                return files
                    .into_iter()
                    .map(|path| {
                        path.into_os_string()
                            .into_string()
                            .map_err(|_| "chat.drop.failed".into())
                    })
                    .collect()
            }
            Ok(_) | Err(arboard::Error::ContentNotAvailable) => {}
            Err(error) => return Err(error.to_string()),
        }
        let image = match clipboard.get_image() {
            Ok(image) => image,
            Err(arboard::Error::ContentNotAvailable) => return Ok(vec![]),
            Err(error) => return Err(error.to_string()),
        };
        const LIMIT: usize = 64 * 1024 * 1024;
        let length = image
            .width
            .checked_mul(image.height)
            .and_then(|n| n.checked_mul(4));
        if image.width == 0
            || image.height == 0
            || length != Some(image.bytes.len())
            || image.bytes.len() > LIMIT
        {
            return Err("chat.drop.failed".into());
        }
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, image.width as u32, image.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&image.bytes)
            .map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
        if png.len() > LIMIT {
            return Err("chat.drop.failed".into());
        }
        let base = self.0.join("attachments");
        std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        let directory = base.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
        let path = directory.join("pasted.png");
        // The application-local directory inherits the Windows user's access controls.
        // Keep successful images for persisted @path references, as on the original desktop.
        if let Err(error) = std::fs::write(&path, png) {
            let _ = std::fs::remove_dir_all(&directory);
            return Err(error.to_string());
        }
        Ok(vec![path
            .into_os_string()
            .into_string()
            .map_err(|_| "chat.drop.failed")?])
    }
}

#[cfg(not(windows))]
impl prometeu_bridge::application::AttachmentClipboard for Clipboard {
    fn files(&self) -> Result<Vec<String>, String> {
        let _ = &self.0;
        Err("Windows clipboard is unavailable on this host".into())
    }
}

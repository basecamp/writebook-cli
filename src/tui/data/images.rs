use anyhow::{Context, Result, bail};

use super::client;
use crate::api;

const IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const IMAGE_SIDE: u32 = 8000;
const IMAGE_MEMORY: u64 = 64 * 1024 * 1024;

/// Where an image in a page comes from.
#[derive(Debug, PartialEq)]
pub enum ImageAt {
    /// A path on the instance, fetched with its token.
    Instance(String),
    /// An address on another site, fetched without it.
    Elsewhere(String),
}

/// Where to fetch an image a page or book shows: its path on the instance,
/// or an address that's on the instance or on another site.
pub fn image_at(host: &str, path: &str) -> Result<ImageAt> {
    let instance = url::Url::parse(host).with_context(|| format!("{host} isn't a web address"))?;
    let url = instance
        .join(path)
        .with_context(|| format!("{path:?} isn't a web address"))?;

    if url.origin() == instance.origin() {
        Ok(ImageAt::Instance(
            url[url::Position::BeforePath..url::Position::AfterQuery].to_string(),
        ))
    } else if matches!(url.scheme(), "http" | "https") {
        Ok(ImageAt::Elsewhere(url.into()))
    } else {
        bail!("{path:?} isn't a web address")
    }
}

pub fn image(host: &str, path: &str) -> Result<image::DynamicImage> {
    let bytes = match image_at(host, path)? {
        ImageAt::Instance(path) => client(host)?.download_at_most(&path, IMAGE_BYTES)?,
        ImageAt::Elsewhere(url) => download_elsewhere(&url)?,
    };

    decode(image::ImageReader::new(std::io::Cursor::new(bytes)))
}

fn download_elsewhere(url: &str) -> Result<Vec<u8>> {
    let mut response = api::agent()
        .get(url)
        .call()
        .with_context(|| format!("could not reach {url}"))?;

    if !response.status().is_success() {
        bail!("{url} answered {}", response.status());
    }

    response
        .body_mut()
        .with_config()
        .limit(IMAGE_BYTES)
        .read_to_vec()
        .with_context(|| format!("could not download {url}"))
}

pub fn image_file(path: &std::path::Path) -> Result<image::DynamicImage> {
    decode(image::ImageReader::open(path).with_context(|| format!("could not read {}", path.display()))?)
}

fn decode<R: std::io::BufRead + std::io::Seek>(reader: image::ImageReader<R>) -> Result<image::DynamicImage> {
    let mut reader = reader.with_guessed_format().context("could not read the image")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(IMAGE_SIDE);
    limits.max_image_height = Some(IMAGE_SIDE);
    limits.max_alloc = Some(IMAGE_MEMORY);
    reader.limits(limits);

    reader.decode().context("could not read the image")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetches_images_on_the_instance_with_its_token_and_others_without() {
        let host = "https://books.example.com";

        assert_eq!(
            image_at(host, "/u/a.png?v=2").unwrap(),
            ImageAt::Instance("/u/a.png?v=2".to_string())
        );
        assert_eq!(
            image_at(host, "https://books.example.com/u/a.png").unwrap(),
            ImageAt::Instance("/u/a.png".to_string())
        );
        assert_eq!(
            image_at(host, "https://cdn.example.org/a.png").unwrap(),
            ImageAt::Elsewhere("https://cdn.example.org/a.png".to_string())
        );
        assert_eq!(
            image_at(host, "//cdn.example.org/a.png").unwrap(),
            ImageAt::Elsewhere("https://cdn.example.org/a.png".to_string())
        );
        assert_eq!(
            image_at(host, "http://books.example.com/u/a.png").unwrap(),
            ImageAt::Elsewhere("http://books.example.com/u/a.png".to_string())
        );
        assert!(image_at(host, "file:///etc/passwd").is_err());
        assert!(image_at(host, "data:image/png;base64,AAAA").is_err());
    }

    #[test]
    fn refuses_to_decode_an_image_too_big_to_show() {
        let mut png = Vec::new();
        image::DynamicImage::new_luma8(IMAGE_SIDE + 1, 1)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        assert!(decode(image::ImageReader::new(std::io::Cursor::new(png))).is_err());
    }
}

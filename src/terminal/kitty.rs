// Copyright 2020 Sebastian Wiesner <sebastian@swsnr.de>
// Copyright 2019 Fabian Spillner <fabian.spillner@gmail.com>

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at

//  http://www.apache.org/licenses/LICENSE-2.0

// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The kitty terminal.
//!
//! kitty is a fast, featureful, GPU based terminal emulator.
//!
//! See <https://sw.kovidgoyal.net/kitty/> for more information.

use crate::resources::read_url;
use crate::svg::render_svg;
use crate::terminal::size::PixelSize;
use crate::{magic, ResourceAccess};
use anyhow::{Context, Error};
use fehler::throws;
use image::imageops::FilterType;
use image::ColorType;
use image::{DynamicImage, GenericImageView};
use std::io::Write;
use std::str;
use url::Url;

/// Whether we run in Kitty or not.
pub fn is_kitty() -> bool {
    std::env::var("TERM")
        .map(|value| value == "xterm-kitty")
        .unwrap_or(false)
}

/// Provides access to printing images for kitty.
#[derive(Debug, Copy, Clone)]
pub struct KittyImages;

impl KittyImages {
    /// Write an inline image for kitty.
    #[throws]
    pub fn write_inline_image<W: Write>(self, writer: &mut W, image: KittyImage) -> () {
        // Kitty's escape sequence is like: Put the command key/value pairs together like "{}={}(,*)"
        // and write them along with the image bytes in 4096 bytes chunks to the stdout.
        // Documentation gives the following python example:
        //
        //  import sys
        //  from base64 import standard_b64encode
        //
        //  def serialize_gr_command(cmd, payload=None):
        //    cmd = ','.join('{}={}'.format(k, v) for k, v in cmd.items())
        //    ans = []
        //    w = ans.append
        //    w(b'\033_G'), w(cmd.encode('ascii'))
        //    if payload:
        //      w(b';')
        //      w(payload)
        //    w(b'\033\\')
        //    return b''.join(ans)
        //
        //  def write_chunked(cmd, data):
        //    cmd = {'a': 'T', 'f': 100}
        //    data = standard_b64encode(data)
        //    while data:
        //      chunk, data = data[:4096], data[4096:]
        //      m = 1 if data else 0
        //      cmd['m'] = m
        //      sys.stdout.buffer.write(serialize_gr_command(cmd, chunk))
        //      sys.stdout.flush()
        //      cmd.clear()
        //
        // Check at <https://sw.kovidgoyal.net/kitty/graphics-protocol.html#control-data-reference>
        // for the reference.
        let mut cmd_header: Vec<String> = vec![
            "a=T".into(),
            "t=d".into(),
            format!("f={}", image.format.control_data_value()),
        ];

        if let Some(size) = image.size {
            cmd_header.push(format!("s={}", size.x));
            cmd_header.push(format!("v={}", size.y));
        }

        let image_data = base64::encode(&image.contents);
        let image_data_chunks = image_data.as_bytes().chunks(4096);
        let image_data_chunks_length = image_data_chunks.len();

        for (i, data) in image_data_chunks.enumerate() {
            if i < image_data_chunks_length - 1 {
                cmd_header.push("m=1".into());
            } else {
                cmd_header.push("m=0".into());
            }

            let cmd = format!(
                "\x1b_G{};{}\x1b\\",
                cmd_header.join(","),
                str::from_utf8(data)?
            );
            writer.write_all(cmd.as_bytes())?;
            writer.flush()?;

            cmd_header.clear();
        }
    }

    /// Read the image bytes from the given URL and wrap them in a `KittyImage`.
    ///
    /// If the image size exceeds `terminal_size` in either dimension scale the
    /// image down to `terminal_size` (preserving aspect ratio).
    #[throws]
    pub fn read_and_render(
        self,
        url: &Url,
        access: ResourceAccess,
        terminal_size: PixelSize,
    ) -> KittyImage {
        let contents = read_url(url, access)?;
        let mime = magic::detect_mime_type(&contents)
            .with_context(|| format!("Failed to detect mime type for URL {}", url))?;
        let image = if magic::is_svg(&mime) {
            image::load_from_memory(
                &render_svg(&contents)
                    .with_context(|| format!("Failed to render SVG at {} to PNG", url))?,
            )
            .with_context(|| format!("Failed to load SVG rendered from {}", url))?
        } else {
            image::load_from_memory(&contents)
                .with_context(|| format!("Failed to load image from URL {}", url))?
        };

        if magic::is_png(&mime) && PixelSize::from_xy(image.dimensions()) <= terminal_size {
            self.render_as_png(contents)
        } else {
            self.render_as_rgb_or_rgba(image, terminal_size)
        }
    }

    /// Wrap the image bytes as PNG format in `KittyImage`.
    fn render_as_png(self, contents: Vec<u8>) -> KittyImage {
        KittyImage {
            contents,
            format: KittyFormat::PNG,
            size: None,
        }
    }

    /// Render the image as RGB/RGBA format and wrap the image bytes in `KittyImage`.
    ///
    /// If the image size exceeds `terminal_size` in either dimension scale the
    /// image down to `terminal_size` (preserving aspect ratio).
    fn render_as_rgb_or_rgba(self, image: DynamicImage, terminal_size: PixelSize) -> KittyImage {
        let format = match image.color() {
            ColorType::L8
            | ColorType::Rgb8
            | ColorType::L16
            | ColorType::Rgb16
            | ColorType::Bgr8 => KittyFormat::RGB,
            // Default to RGBA format: We cannot match all colour types because
            // ColorType is marked non-exhaustive, but RGBA is a safe default
            // because we can convert any image to RGBA, at worth with additional
            // runtime costs.
            _ => KittyFormat::RGBA,
        };

        let image = if PixelSize::from_xy(image.dimensions()) <= terminal_size {
            image
        } else {
            image.resize(
                terminal_size.x as u32,
                terminal_size.y as u32,
                FilterType::Nearest,
            )
        };

        let size = PixelSize::from_xy(image.dimensions());

        KittyImage {
            contents: match format {
                KittyFormat::RGB => image.into_rgb().into_raw(),
                _ => image.into_rgba().into_raw(),
            },
            format,
            size: Some(size),
        }
    }
}

/// Holds the image bytes with its image format and dimensions.
pub struct KittyImage {
    contents: Vec<u8>,
    format: KittyFormat,
    size: Option<PixelSize>,
}

/// The image format (PNG, RGB or RGBA) of the image bytes.
enum KittyFormat {
    PNG,
    RGB,
    RGBA,
}

impl KittyFormat {
    /// Return the control data value of the image format.
    /// See the [documentation] for the reference and explanation.
    ///
    /// [documentation]: https://sw.kovidgoyal.net/kitty/graphics-protocol.html#transferring-pixel-data
    fn control_data_value(&self) -> &str {
        match *self {
            KittyFormat::PNG => "100",
            KittyFormat::RGB => "24",
            KittyFormat::RGBA => "32",
        }
    }
}

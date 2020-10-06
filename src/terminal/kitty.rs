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
use crate::{magic, ResourceAccess};
use anyhow::{anyhow, Context, Error, Result};
use fehler::throws;
use image::imageops::FilterType;
use image::ColorType;
use image::{DynamicImage, GenericImageView};
use std::io::Write;
use std::process::{Command, Stdio};
use std::str;
use url::Url;

/// Whether we run in Kitty or not. It also returns the version.
pub fn is_kitty() -> Option<(u8, u8, u8)> {
    std::env::var("TERM")
        .ok()
        .filter(|value| value == "xterm-kitty")?;

    let output = Command::new("kitty").arg("--version").output().ok()?;
    if output.status.success() {
        // Output is in the form of `kitty <major>.<minor>.<patch> created...`.
        let output = std::str::from_utf8(&output.stdout).ok()?;
        let mut version = output.split_ascii_whitespace().nth(1)?.split('.');
        let major = version.next()?.parse().ok()?;
        let minor = version.next()?.parse().ok()?;
        let patch = version.next()?.parse().ok()?;
        Some((major, minor, patch))
    } else {
        None
    }
}

/// Retrieve the terminal size in pixels by calling the command-line tool `kitty`.
///
/// ```console
/// $ kitty +kitten icat --print-window-size
/// ```
///
/// We cannot use the terminal size information from Context.output.size, because
/// the size information are in columns / rows instead of pixel.
fn get_terminal_size() -> Result<KittyDimension> {
    let process = Command::new("kitty")
        .arg("+kitten")
        .arg("icat")
        .arg("--print-window-size")
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| "Failed to spawn kitty +kitten icat --print-window-size")?;

    let output = process.wait_with_output()?;

    if output.status.success() {
        let terminal_size_str = std::str::from_utf8(&output.stdout).with_context(|| {
            format!(
                "kitty +kitten icat --print-window-size returned non-utf8: {:?}",
                output.stdout
            )
        })?;
        let terminal_size = terminal_size_str.split('x').collect::<Vec<&str>>();

        terminal_size[0]
            .parse::<u32>()
            .and_then(|width| {
                terminal_size[1]
                    .parse::<u32>()
                    .map(|height| KittyDimension { width, height })
            })
            .with_context(|| {
                format!(
                    "Failed to parse kitty width and height from output: {}",
                    terminal_size_str
                )
            })
    } else {
        Err(anyhow!(
            "kitty +kitten icat --print-window-size failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ))
    }
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

        if let Some(dimension) = image.dimension {
            cmd_header.push(format!("s={}", dimension.width));
            cmd_header.push(format!("v={}", dimension.height));
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
    /// It scales the image down, if the image size exceeds the terminal window size.
    #[throws]
    pub fn read_and_render(self, url: &Url, access: ResourceAccess) -> KittyImage {
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
        let terminal_size = get_terminal_size()?;

        if magic::is_png(&mime) && terminal_size.contains(&image.dimensions().into()) {
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
            dimension: None,
        }
    }

    /// Render the image as RGB/RGBA format and wrap the image bytes in `KittyImage`.
    /// It scales the image down if its size exceeds the terminal size.
    fn render_as_rgb_or_rgba(
        self,
        image: DynamicImage,
        terminal_size: KittyDimension,
    ) -> KittyImage {
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

        let image = if terminal_size.contains(&KittyDimension::from(image.dimensions())) {
            image
        } else {
            image.resize(
                terminal_size.width,
                terminal_size.height,
                FilterType::Nearest,
            )
        };

        let image_dimension = image.dimensions().into();

        KittyImage {
            contents: match format {
                KittyFormat::RGB => image.into_rgb().into_raw(),
                _ => image.into_rgba().into_raw(),
            },
            format,
            dimension: Some(image_dimension),
        }
    }
}

/// Holds the image bytes with its image format and dimensions.
pub struct KittyImage {
    contents: Vec<u8>,
    format: KittyFormat,
    dimension: Option<KittyDimension>,
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

/// The dimension encapsulate the width and height in the pixel unit.
struct KittyDimension {
    width: u32,
    height: u32,
}

impl KittyDimension {
    /// Check whether this dimension entirely contains the specified dimension.
    fn contains(&self, other: &KittyDimension) -> bool {
        self.width >= other.width && self.height >= other.height
    }
}

impl From<(u32, u32)> for KittyDimension {
    /// Convert a tuple struct (`u32`, `u32`) ordered by width and height
    /// into a `KittyDimension`.
    fn from(dimension: (u32, u32)) -> KittyDimension {
        let (width, height) = dimension;

        KittyDimension { width, height }
    }
}

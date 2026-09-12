use std::{
    env, fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use image::{DynamicImage, ImageFormat, ImageReader, RgbaImage, imageops};

const ICON_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256, 512, 1024];

pub fn install_user_icon(app_id: &str, icon: Option<&Path>) -> Result<Option<String>, String> {
    let Some(icon) = icon.filter(|icon| icon.is_file()) else {
        return Ok(None);
    };
    let hicolor = data_home()?.join("icons/hicolor");
    install_icon_set(app_id, icon, &hicolor)?;
    refresh_icon_cache(&hicolor);
    Ok(Some(app_id.to_string()))
}

pub fn stage_icon_set(app_id: &str, icon: &Path, destination: &Path) -> Result<(), String> {
    if !icon.is_file() {
        return Ok(());
    }
    install_icon_set(app_id, icon, destination)
}

pub fn stage_windows_icon(
    app_id: &str,
    icon: &Path,
    icon_set: &Path,
    destination: &Path,
) -> Result<(), String> {
    if icon
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ico"))
    {
        return copy_file(icon, destination);
    }
    let sizes = [16_u32, 24, 32, 48, 64, 128, 256];
    let images = sizes
        .iter()
        .map(|size| {
            fs::read(icon_set.join(format!("{size}x{size}/apps/{app_id}.png")))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut offset = 6 + 16 * sizes.len() as u32;
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    for (size, png) in sizes.iter().zip(&images) {
        ico.extend_from_slice(&[*size as u8, *size as u8, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in images {
        ico.extend_from_slice(&png);
    }

    copy_bytes(&ico, destination)
}

pub fn stage_macos_icon(app_id: &str, icon_set: &Path, destination: &Path) -> Result<(), String> {
    use icns::{IconFamily, IconType, PixelFormat};
    let mut family = IconFamily::new();
    let types = [
        (16, IconType::RGB24_16x16),
        (32, IconType::RGB24_32x32),
        (32, IconType::RGBA32_16x16_2x),
        (64, IconType::RGBA32_32x32_2x),
        (128, IconType::RGBA32_128x128),
        (256, IconType::RGBA32_128x128_2x),
        (256, IconType::RGBA32_256x256),
        (512, IconType::RGBA32_256x256_2x),
        (512, IconType::RGBA32_512x512),
        (1024, IconType::RGBA32_512x512_2x),
    ];
    for (size, kind) in types {
        let source = icon_set.join(format!("{size}x{size}/apps/{app_id}.png"));
        let image = load_raster(&source)?.into_rgba8();
        let image = icns::Image::from_data(PixelFormat::RGBA, size, size, image.into_raw())
            .map_err(|error| error.to_string())?;
        family
            .add_icon_with_type(&image, kind)
            .map_err(|error| error.to_string())?;
    }
    let mut data = Vec::new();
    family.write(&mut data).map_err(|error| error.to_string())?;
    copy_bytes(&data, destination)
}

fn install_icon_set(app_id: &str, icon: &Path, root: &Path) -> Result<(), String> {
    let image = if extension(icon)?.eq_ignore_ascii_case("svg") {
        copy_file(
            icon,
            &root.join("scalable/apps").join(format!("{app_id}.svg")),
        )?;
        let output = Command::new("magick")
            .args(["-background", "none", "-density", "384"])
            .arg(icon)
            .args(["-resize", "1024x1024", "png:-"])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("SVG icons require ImageMagick: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "failed to rasterize icon {}: {}",
                icon.display(),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        image::load_from_memory_with_format(&output.stdout, ImageFormat::Png)
            .map_err(|error| format!("failed to decode rasterized SVG: {error}"))?
    } else {
        load_raster(icon)?
    };
    for size in ICON_SIZES {
        let destination = root.join(format!("{size}x{size}/apps/{app_id}.png"));
        copy_bytes(&encode_png(&square_icon(&image, *size))?, &destination)?;
    }
    Ok(())
}

fn load_raster(path: &Path) -> Result<DynamicImage, String> {
    ImageReader::open(path)
        .map_err(|error| format!("failed to open icon {}: {error}", path.display()))?
        .with_guessed_format()
        .map_err(|error| format!("failed to identify icon {}: {error}", path.display()))?
        .decode()
        .map_err(|error| format!("failed to decode icon {}: {error}", path.display()))
}

fn square_icon(image: &DynamicImage, size: u32) -> DynamicImage {
    let image = image
        .resize(size, size, imageops::FilterType::Lanczos3)
        .into_rgba8();
    let mut canvas = RgbaImage::new(size, size);
    imageops::overlay(
        &mut canvas,
        &image,
        i64::from((size - image.width()) / 2),
        i64::from((size - image.height()) / 2),
    );
    DynamicImage::ImageRgba8(canvas)
}

fn encode_png(image: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|error| format!("failed to encode icon: {error}"))?;
    Ok(output.into_inner())
}

fn copy_file(source: &Path, destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::copy(source, destination).map_err(|error| error.to_string())?;
    Ok(())
}

fn copy_bytes(bytes: &[u8], destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(destination, bytes).map_err(|error| error.to_string())
}

fn extension(path: &Path) -> Result<String, String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_string)
        .ok_or_else(|| format!("icon path has no extension: {}", path.display()))
}

fn refresh_icon_cache(root: &Path) {
    if !root.exists() {
        return;
    }
    let _ = Command::new("gtk-update-icon-cache")
        .args(["-q", "-t"])
        .arg(root)
        .status();
}

fn data_home() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(path));
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".local/share"))
        .ok_or_else(|| "HOME is not set".to_string())
}

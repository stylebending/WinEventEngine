use std::env;
use std::path::PathBuf;

fn main() {
    // Only run on Windows
    if env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "windows" {
        println!("cargo:warning=Icon embedding only supported on Windows");
        return;
    }

    // Get paths - logo is in root assets folder
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let manifest_path = PathBuf::from(&manifest_dir);
    let root_dir = manifest_path.parent().unwrap(); // Go up from gui/ to root
    let assets_dir = root_dir.join("assets");
    let png_path = assets_dir.join("logo.png");
    let ico_path = manifest_path.join("assets").join("logo.ico");

    // Check if logo.png exists
    if !png_path.exists() {
        println!("cargo:warning=Logo not found at {:?}", png_path);
        return;
    }

    // Generate resized logo (256x256) for runtime use to save memory
    let resized_png_path = assets_dir.join("logo_256.png");
    if !resized_png_path.exists() || is_newer(&png_path, &resized_png_path) {
        println!("cargo:warning=Generating 256x256 logo for memory optimization...");
        match generate_resized_logo(&png_path, &resized_png_path, 256) {
            Ok(_) => println!("cargo:warning=Resized logo generated successfully"),
            Err(e) => println!("cargo:warning=Failed to generate resized logo: {}", e),
        }
    }

    // Generate ICO from PNG if needed
    if !ico_path.exists() || is_newer(&png_path, &ico_path) {
        println!("cargo:warning=Generating ICO file from PNG...");
        match generate_ico(&png_path, &ico_path) {
            Ok(_) => println!("cargo:warning=ICO generated successfully"),
            Err(e) => println!("cargo:warning=Failed to generate ICO: {}", e),
        }
    }

    // Compile resource file with just the icon
    let mut res = winres::WindowsResource::new();

    // Set icon directly - this embeds it into the executable
    if ico_path.exists() {
        res.set_icon(ico_path.to_str().unwrap());
        println!("cargo:warning=Setting icon from {:?}", ico_path);
    }

    // Compile
    match res.compile() {
        Ok(_) => println!("cargo:warning=Resources compiled successfully"),
        Err(e) => println!("cargo:warning=Resource compilation warning: {}", e),
    }

    // Tell cargo to rebuild if logo changes (in root assets folder)
    println!("cargo:rerun-if-changed=../../assets/logo.png");
}

fn generate_resized_logo(
    png_path: &PathBuf,
    output_path: &PathBuf,
    size: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    // Load PNG
    let img = image::open(png_path)?;

    // Resize to specified dimensions
    let resized = img.resize(size, size, image::imageops::FilterType::Lanczos3);

    // Save as PNG
    resized.save(output_path)?;

    Ok(())
}

fn generate_ico(png_path: &PathBuf, ico_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // Load PNG
    let img = image::open(png_path)?;

    // Create ICO with multiple resolutions
    let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);

    // Standard Windows icon sizes
    let sizes = [16, 32, 48, 256];

    for size in sizes {
        // Resize image
        let resized = img.resize(size, size, image::imageops::FilterType::Lanczos3);

        // Convert to RGBA
        let rgba = resized.to_rgba8();
        let (width, height) = rgba.dimensions();

        // Create IconImage from raw RGBA data
        let icon_image = ico::IconImage::from_rgba_data(width, height, rgba.into_raw());

        // Encode as PNG for best quality and transparency support
        let icon = ico::IconDirEntry::encode_as_png(&icon_image)?;
        icon_dir.add_entry(icon);
    }

    // Write ICO file
    let file = std::fs::File::create(ico_path)?;
    icon_dir.write(file)?;

    Ok(())
}

fn is_newer(file1: &PathBuf, file2: &PathBuf) -> bool {
    let meta1 = std::fs::metadata(file1).unwrap();
    let meta2 = std::fs::metadata(file2).unwrap();

    let time1 = meta1.modified().unwrap();
    let time2 = meta2.modified().unwrap();

    time1 > time2
}

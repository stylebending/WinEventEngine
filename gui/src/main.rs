#![windows_subsystem = "windows"]

use iced::{Settings, Size};
use std::env;

mod app;
mod components;
mod theme;
mod views;

use app::WinEventApp;

#[cfg(target_os = "windows")]
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS, FreeConsole};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Load the window icon from embedded assets (uses pre-resized 256x256 to save memory)
fn load_window_icon() -> Option<iced::window::Icon> {
    // Use the pre-resized 256x256 logo (generated at build time)
    // This saves ~3.5MB compared to loading the full 1024x1024 image
    let icon_bytes = include_bytes!("../../assets/logo_256.png");
    
    // Load the image using the image crate
    match image::load_from_memory_with_format(icon_bytes, image::ImageFormat::Png) {
        Ok(img) => {
            // Convert to RGBA
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            
            // Create iced icon
            match iced::window::icon::from_rgba(rgba.into_raw(), width, height) {
                Ok(icon) => Some(icon),
                Err(e) => {
                    eprintln!("Failed to create window icon: {}", e);
                    None
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to load logo image: {}", e);
            None
        }
    }
}

fn print_help() {
    println!("WinEventEngine v{}", VERSION);
    println!("A universal event automation system for Windows");
    println!();
    println!("USAGE:");
    println!("    WinEventEngine.exe [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    --cli          Run in CLI mode (terminal output)");
    println!("    --version      Show version information");
    println!("    --help         Show this help message");
    println!();
    println!("EXAMPLES:");
    println!("    WinEventEngine.exe           Run the GUI");
    println!("    WinEventEngine.exe --cli     Run in CLI mode");
}

fn print_version() {
    println!("WinEventEngine v{}", VERSION);
}

#[tokio::main]
async fn main() {
    // Check for CLI mode
    let args: Vec<String> = env::args().collect();
    
    // Handle --help and --version before anything else
    if args.contains(&"--help".to_string()) || args.contains(&"-h".to_string()) {
        // Attach to console for output
        #[cfg(target_os = "windows")]
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
        print_help();
        return;
    }
    
    if args.contains(&"--version".to_string()) || args.contains(&"-V".to_string()) {
        // Attach to console for output
        #[cfg(target_os = "windows")]
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
        print_version();
        return;
    }

    if args.contains(&"--cli".to_string()) {
        // Attach to parent console so output appears in terminal
        #[cfg(target_os = "windows")]
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
        
        // Run CLI mode
        if let Err(e) = win_event_engine::run_cli().await {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    // Run GUI mode
    let result = iced::application("WinEventEngine", WinEventApp::update, WinEventApp::view)
        .settings(Settings {
            default_font: iced::Font::DEFAULT,
            default_text_size: iced::Pixels(16.0),
            ..Settings::default()
        })
        .window(iced::window::Settings {
            size: Size::new(1200.0, 800.0),
            min_size: Some(Size::new(900.0, 600.0)),
            resizable: true,
            decorations: true,
            transparent: false,
            icon: load_window_icon(),
            ..iced::window::Settings::default()
        })
        .theme(|app| app.theme())
        .subscription(|_| iced::time::every(std::time::Duration::from_millis(100)).map(|_| app::Message::Tick))
        .run_with(WinEventApp::new);
    
    if let Err(e) = result {
        eprintln!("GUI error: {}", e);
        std::process::exit(1);
    }
}

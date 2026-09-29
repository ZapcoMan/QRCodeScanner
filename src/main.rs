use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "qr-scanner")]
#[command(about = "A command-line QR code scanner", long_about = None)]
struct Args {
    #[arg(help = "Path to the QR code image file")]
    image_path: PathBuf,
}

fn main() {
    let args = Args::parse();

    if !args.image_path.exists() {
        eprintln!("Error: File not found: {}", args.image_path.display());
        std::process::exit(1);
    }

    println!("Scanning QR code from: {}", args.image_path.display());

    let img = match image::open(&args.image_path) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("Error loading image: {}", e);
            std::process::exit(1);
        }
    };

    let results = bardecoder::default_decoder().decode(&img);
    
    let mut found = false;
    for result in results {
        match result {
            Ok(text) => {
                found = true;
                println!("{}", text);
            }
            Err(e) => {
                eprintln!("Error decoding QR code: {}", e);
            }
        }
    }

    if !found {
        println!("No QR code found in the image.");
    }
}
use std::num::NonZeroUsize;

use clap::Parser;
use color_eyre::Result;
use filesystem::ZipFs;
use fuser::MountOption;
use tracing::{debug, info};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod file_tree;
mod filesystem;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(index = 1)]
    data_dir: std::path::PathBuf,

    #[arg(index = 2)]
    mount_point: std::path::PathBuf,

    #[arg(short, long, default_value_t = NonZeroUsize::new(1024).unwrap())]
    cache_size: NonZeroUsize,
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();

    let filter = EnvFilter::builder()
        .with_default_directive("zipfs=info".parse()?)
        .from_env_lossy();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();

    let (tx, rx) = std::sync::mpsc::channel();

    info!("Mounting ZIP file system");
    info!("Data directory: {:?}", args.data_dir);
    info!("Mount point: {:?}", args.mount_point);
    info!("Cache size: {}", args.cache_size);
    let guard = fuser::spawn_mount2(
        ZipFs::new(args.data_dir, args.cache_size, tx.clone()),
        args.mount_point,
        &[MountOption::RO],
    )?;

    ctrlc::set_handler(move || {
        debug!("Received signal to unmount");
        tx.send(()).unwrap();
    })?;

    // NOTE: Drop the guard only after we have received a signal
    rx.recv()?;
    drop(guard);
    info!("Successfully unmounted");

    Ok(())
}

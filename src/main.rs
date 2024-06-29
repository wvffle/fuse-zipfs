use std::num::NonZeroUsize;

use clap::Parser;
use color_eyre::Result;
use fuse3::{path::Session, raw::MountHandle, MountOptions};
use tokio::signal;
use tracing::info;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use zipfs::ZipFs;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(index = 1)]
    data_dir: std::path::PathBuf,

    #[arg(index = 2)]
    mount_point: std::path::PathBuf,

    #[arg(short, long, default_value_t = NonZeroUsize::new(1024).unwrap())]
    cache_size: NonZeroUsize,

    #[arg(short = 'o', long, default_value_t = String::from("ro"))]
    mount_options: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let filter = EnvFilter::builder()
        .with_default_directive("zipfs=debug".parse()?)
        .from_env_lossy();

    let layer = fmt::layer()
        .pretty()
        .with_target(true)
        .with_writer(std::io::stderr);

    tracing_subscriber::registry()
        .with(layer)
        .with(filter)
        .init();

    let mut mount_handle = mount().await?;
    let handle = &mut mount_handle;

    // BUG: handle does not finish when externally unmounted
    // see upstream: https://github.com/Sherlock-Holo/fuse3/issues/102
    tokio::select! {
        res = handle => res?,
        _ = signal::ctrl_c() => mount_handle.unmount().await?
    }

    info!("Successfully unmounted");

    Ok(())
}

fn apply_options(options: String, mount_options: &mut MountOptions) {
    mount_options
        .fs_name("zipfs")
        .read_only(true)
        .force_readdir_plus(true);

    for opt in options.split(',') {
        match opt {
            "default_permissions" => mount_options.default_permissions(true),
            "allow_other" => mount_options.allow_other(true),
            "allow_root" => mount_options.allow_root(true),
            other => mount_options.custom_options(other),
        };
    }
}

async fn mount() -> Result<MountHandle> {
    let args = Args::parse();

    info!("Mounting ZIP file system");
    info!("Data directory: {:?}", args.data_dir);
    info!("Mount point: {:?}", args.mount_point);
    info!("Cache size: {}", args.cache_size);

    let mut mount_options = MountOptions::default();
    apply_options(args.mount_options, &mut mount_options);

    let handle = Session::new(mount_options)
        .mount_with_unprivileged(ZipFs::new(args.data_dir, args.cache_size), args.mount_point)
        .await?;

    Ok(handle)
}

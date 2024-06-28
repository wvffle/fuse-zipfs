use std::num::NonZeroUsize;

use clap::Parser;
use color_eyre::Result;
use fuse3::{path::Session, MountOptions};
use tracing::{debug, info};
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

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();

    let filter = EnvFilter::builder()
        .with_default_directive("zipfs=info".parse()?)
        .from_env_lossy();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();

    // let (tx, rx) = std::sync::mpsc::channel();

    info!("Mounting ZIP file system");
    info!("Data directory: {:?}", args.data_dir);
    info!("Mount point: {:?}", args.mount_point);
    info!("Cache size: {}", args.cache_size);

    let mut mount_options = MountOptions::default();
    mount_options
        .fs_name("zipfs")
        .read_only(true)
        .force_readdir_plus(true);

    Session::new(mount_options)
        .mount(
            // ZipFs::new(args.data_dir, args.cache_size, Some(tx.clone())),
            ZipFs::new(args.data_dir, args.cache_size, None),
            args.mount_point,
        )
        .await?
        .await?;

    // ctrlc::set_handler(move || {
    //     debug!("Received signal to unmount");
    //     tx.send(()).unwrap();
    // })?;

    // // NOTE: Drop the guard only after we have received a signal
    // rx.recv()?;
    // drop(guard);
    info!("Successfully unmounted");

    Ok(())
}

// fn get_options(mount_options: String) -> Vec<MountOption> {
//     let mut options = vec![MountOption::RO, MountOption::FSName("zipfs".to_string())];
//
//     for opt in mount_options.split(',') {
//         let opt = match opt {
//             "default_permissions" => MountOption::DefaultPermissions,
//             "allow_other" => MountOption::AllowOther,
//             "allow_root" => MountOption::AllowRoot,
//             "auto_unmount" => MountOption::AutoUnmount,
//             "dev" => MountOption::Dev,
//             "nodev" => MountOption::NoDev,
//             "suid" => MountOption::Suid,
//             "nosuid" => MountOption::NoSuid,
//             "atime" => MountOption::Atime,
//             "noatime" => MountOption::NoAtime,
//             "exec" => MountOption::Exec,
//             "noexec" => MountOption::NoExec,
//             other => MountOption::CUSTOM(other.to_string()),
//         };
//
//         options.push(opt);
//     }
//
//     options
// }

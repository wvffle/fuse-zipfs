use std::{fs, path::PathBuf};

use color_eyre::Result;
use fuser::BackgroundSession;
use rstest::rstest;
use temp_dir::TempDir;
use zipfs::ZipFs;

const DATA_DIR: &str = "tests/data";

fn mount() -> Result<(TempDir, BackgroundSession)> {
    let mnt = TempDir::new()?;

    let guard = fuser::spawn_mount2(
        ZipFs::new(PathBuf::from(DATA_DIR), None),
        mnt.path(),
        &[fuser::MountOption::RO],
    )?;

    Ok((mnt, guard))
}

#[test]
fn test_mount() -> Result<()> {
    let (_mnt, guard) = mount()?;
    drop(guard);
    Ok(())
}

#[test]
fn test_readdir_passthrough() -> Result<()> {
    let data = PathBuf::from(DATA_DIR);
    let (mnt, guard) = mount()?;

    let entries_data: Vec<_> = fs::read_dir(data)?.map(|e| e.unwrap()).collect();
    let entries_mnt: Vec<_> = fs::read_dir(mnt.path())?.map(|e| e.unwrap()).collect();

    assert_eq!(entries_mnt.len(), entries_data.len());

    let names_data = entries_data
        .into_iter()
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();

    let names_mnt = entries_mnt
        .into_iter()
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();

    assert_eq!(names_data, names_mnt);

    drop(guard);
    Ok(())
}

#[test]
fn test_read_passthrough() -> Result<()> {
    let data = PathBuf::from(DATA_DIR);
    let (mnt, guard) = mount()?;

    let content_data = fs::read_to_string(data.join("passthrough.txt"))?;
    let content_mnt = fs::read_to_string(mnt.path().join("passthrough.txt"))?;

    assert_eq!(content_data, content_mnt);

    drop(guard);
    Ok(())
}

#[rstest]
#[case::stored("stored.zip")]
#[case::compressed("compressed.zip")]
#[case::encrypted("encrypted.zip")]
fn test_readdir_zip(#[case] zip: &str) -> Result<()> {
    let (mnt, guard) = mount()?;

    let entries: Vec<_> = fs::read_dir(mnt.path().join(zip))?
        .map(|e| e.unwrap())
        .collect();

    let names = entries
        .into_iter()
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["some"]);

    let entries: Vec<_> = fs::read_dir(mnt.path().join(zip).join("some"))?
        .map(|e| e.unwrap())
        .collect();

    let names = entries
        .into_iter()
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["nested"]);

    let entries: Vec<_> = fs::read_dir(mnt.path().join(zip).join("some/nested"))?
        .map(|e| e.unwrap())
        .collect();

    let names = entries
        .into_iter()
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["file.txt"]);

    drop(guard);
    Ok(())
}

#[rstest]
#[case::stored("stored.zip")]
#[case::compressed("compressed.zip")]
fn test_read_zip(#[case] zip: &str) -> Result<()> {
    let (mnt, guard) = mount()?;

    let content = fs::read_to_string(mnt.path().join(zip).join("some/nested/file.txt"))?;
    assert_eq!(content, "some content\n".to_string().repeat(15));

    drop(guard);
    Ok(())
}

#[rstest]
#[case::stored("stored.zip")]
#[case::compressed("compressed.zip")]
#[case::encrypted("encrypted.zip")]
fn test_encrypted_zip_mounts_dirs(#[case] zip: &str) -> Result<()> {
    let (mnt, guard) = mount()?;

    let meta = fs::metadata(mnt.path().join(zip).join("some/nested/file.txt"))?;
    assert_eq!(meta.is_dir(), zip == "encrypted.zip");

    drop(guard);
    Ok(())
}

#[test]
fn test_corrupt_zip_mounts_dir() -> Result<()> {
    let (mnt, guard) = mount()?;

    let meta = fs::metadata(mnt.path().join("corrupt.zip"))?;
    assert!(meta.is_dir());

    drop(guard);
    Ok(())
}

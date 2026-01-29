use std::{path::PathBuf, sync::Arc};

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

#[rstest]
#[case::stored("stored.zip")]
#[case::compressed("compressed.zip")]
fn test_concurrent_readdir_and_read_dont_conflict(#[case] zip: &str) -> Result<()> {
    let (mnt, guard) = mount()?;

    let (zip_dir, nested_dir, file_path) = (
        mnt.path().join(zip).join("some"),
        mnt.path().join(zip).join("some/nested"),
        mnt.path().join(zip).join("some/nested/file.txt"),
    );

    let thread_count = 20;

    let results = Arc::new(std::sync::Mutex::new(Vec::new()));

    let threads: Vec<_> = (0..thread_count)
        .map(|_i| {
            let result = Arc::clone(&results);
            let zip_dir = zip_dir.clone();
            let nested_dir = nested_dir.clone();
            let file_path = file_path.clone();

            std::thread::spawn(move || {
                let file_content = std::fs::read_to_string(&file_path).unwrap();
                let entries: Vec<_> = std::fs::read_dir(&nested_dir)
                    .unwrap()
                    .map(|e| e.unwrap().file_name())
                    .collect();
                assert!(file_content.contains("some content"));
                assert!(!entries.is_empty());
                result
                    .lock()
                    .unwrap()
                    .push((file_content.clone(), entries.clone()));
                let zip_entries: Vec<_> = std::fs::read_dir(&zip_dir)
                    .unwrap()
                    .map(|e| e.unwrap().file_name())
                    .collect();
                result.lock().unwrap().push((file_content, zip_entries));
            })
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }

    let all_results = results.lock().unwrap().clone();

    let first_result = all_results[0].clone();
    for result in &all_results[1..] {
        assert_eq!(
            first_result.0, result.0,
            "File content should remain consistent across all threads"
        );
        assert_eq!(
            first_result.0, result.0,
            "Directory listing should remain consistent"
        );
    }

    drop(guard);
    Ok(())
}

#[rstest]
#[case::stored("stored.zip")]
#[case::compressed("compressed.zip")]
fn test_mixed_accesses_same_file_multiple_times(#[case] zip: &str) -> Result<()> {
    let (mnt, guard) = mount()?;

    let file_path = mnt.path().join(zip).join("some/nested/file.txt");

    for _i in 0..50 {
        let meta1 = std::fs::metadata(&file_path).unwrap();
        let content1 = std::fs::read_to_string(&file_path).unwrap();
        let meta2 = std::fs::metadata(&file_path).unwrap();
        let content2 = std::fs::read_to_string(&file_path).unwrap();

        assert_eq!(meta1.len(), meta2.len());
        assert!(meta1.is_file());
        assert!(meta2.is_file());
        assert!(content1.contains("some content"));
        assert_eq!(
            content1.len(),
            content2.len(),
            "File size should remain consistent across all reads"
        );
    }

    drop(guard);
    Ok(())
}

#[rstest]
#[case::stored("stored.zip")]
#[case::compressed("compressed.zip")]
#[case::encrypted("encrypted.zip")]
fn test_concurrent_directory_walks(#[case] zip: &str) -> Result<()> {
    let (mnt, guard) = mount()?;

    let thread_count = 10;
    let paths = vec![
        mnt.path().join(zip).join("some"),
        mnt.path().join(zip).join("some/nested"),
        mnt.path().join(zip),
    ];

    let results = Arc::new(std::sync::Mutex::new(Vec::new()));
    let paths = paths.clone();

    let threads: Vec<_> = (0..thread_count)
        .map(|_i| {
            let result = Arc::clone(&results);
            let paths = paths.clone();

            std::thread::spawn(move || {
                let mut all_entries = Vec::new();
                for path in &paths {
                    if let Ok(entries) = std::fs::read_dir(path) {
                        for entry in entries.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            let metadata = entry.metadata().unwrap();

                            let is_dir = metadata.is_dir();
                            let is_file = metadata.is_file();

                            all_entries.push((name, is_dir, is_file));
                        }
                    }
                }
                result.lock().unwrap().push(all_entries);
            })
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }

    let all_results = results.lock().unwrap().clone();

    let first_result = all_results[0].clone();
    for result in &all_results[1..] {
        assert_eq!(result.len(), first_result.len());
        for (a, b) in result.iter().zip(first_result.iter()) {
            assert_eq!(a, b);
        }
    }

    drop(guard);
    Ok(())
}

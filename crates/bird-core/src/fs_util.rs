use std::io::Write;
use std::path::Path;

/// Replace `path` with `payload` atomically: the bytes go to a temporary file in the
/// same directory, which is then renamed over the target. A concurrent reader or a
/// crash mid-write sees either the old file or the new one, never a truncated one.
pub(crate) fn write_atomic(path: &Path, payload: &[u8]) -> anyhow::Result<()> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(payload)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_atomic;

    #[test]
    fn replaces_existing_file_and_leaves_no_temporary_files() {
        let temp = tempfile::tempdir().expect("temp directory");
        let path = temp.path().join("nested").join("cache.json");

        write_atomic(&path, b"first").expect("first write");
        write_atomic(&path, b"second").expect("second write");

        assert_eq!(std::fs::read(&path).expect("read"), b"second");
        let entries = std::fs::read_dir(path.parent().expect("parent"))
            .expect("list")
            .count();
        assert_eq!(entries, 1);
    }

    #[test]
    fn failed_write_keeps_the_previous_contents() {
        let temp = tempfile::tempdir().expect("temp directory");
        let path = temp.path().join("cache.json");
        write_atomic(&path, b"good").expect("write");

        // A directory in the way makes the final rename fail.
        let blocked = temp.path().join("blocked");
        std::fs::create_dir(&blocked).expect("dir");
        std::fs::write(blocked.join("child"), b"x").expect("child");
        assert!(write_atomic(&blocked, b"bad").is_err());

        assert_eq!(std::fs::read(&path).expect("read"), b"good");
        assert!(blocked.join("child").exists());
    }
}

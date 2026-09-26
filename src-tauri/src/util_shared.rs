use sha2::{Digest, Sha256};
use std::{cmp::Ordering, io::Read};

pub fn sha256(path: &Path) -> Result<String, String> {
    let file =
        fs::File::open(path).map_err(|error| format!("读取 {} 失败：{error}", path.display()))?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn compare_versions(left: &str, right: &str) -> Ordering {
    let parse = |version: &str| {
        version
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let left = parse(left);
    let right = parse(right);
    (0..left.len().max(right.len()))
        .map(|index| {
            left.get(index)
                .unwrap_or(&0)
                .cmp(right.get(index).unwrap_or(&0))
        })
        .find(|order| *order != Ordering::Equal)
        .unwrap_or(Ordering::Equal)
}

pub fn replace_file(stage: &Path, target: &Path) -> Result<(), String> {
    fs::rename(stage, target).map_err(|error| format!("替换 {} 失败：{error}", target.display()))
}

#[cfg(test)]
mod file_tests {
    use super::*;

    #[test]
    fn replacement_overwrites_existing_file_and_failure_preserves_it() {
        let root = std::env::temp_dir().join(format!(
            "claude-file-replace-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let stage = root.join("stage");
        let target = root.join("target");
        fs::write(&target, b"old").unwrap();
        fs::write(&stage, b"new").unwrap();
        replace_file(&stage, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(!stage.exists());
        assert!(replace_file(&stage, &target).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }
}
